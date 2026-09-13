//! Scrapling engine orchestration: a bundled-Python child over anonymous pipes,
//! a native loopback relay that owns every egress decision, and the read-only
//! snapshot store the WebKit view renders from. No credentials ever reach the
//! engine; the relay tokens are per-session capabilities scoped to the relay.
use crate::types::{AppError, AppResult};
use reqwest::Url;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs,
    net::IpAddr,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::{Child, ChildStdin, ChildStdout},
    sync::{mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(super) const CHROMIUM_REVISION: &str = "1234";
pub(super) const CHROMIUM_KIND: &str = "chromium-headless-shell";
pub(crate) const SNAPSHOT_SCHEME: &str = "forma-snapshot";
const MAX_REQUEST: usize = 600_000;
const MAX_RESPONSE: usize = 12 * 1024 * 1024;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(75);
const SNAPSHOT_LIMIT: usize = 50;
const MAX_SNAPSHOT_HTML: usize = 8 * 1024 * 1024;

fn engine_error(message: &'static str) -> AppError {
    AppError::new("browser_engine_unavailable", message)
}

fn assets() -> AppError {
    engine_error("The Scrapling engine resources could not be verified. No engine was started.")
}

// ---------------------------------------------------------------------------
// Egress relay: the only network authority the engine's Python and Chromium
// ever talk to. Fail closed on hostname, port, and every resolved address.
// ---------------------------------------------------------------------------

pub(super) fn denied_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host.contains("higgsfield")
        || host == "localhost"
        || host.ends_with(".localhost")
        || host == "local"
        || host.ends_with(".local")
        || host.ends_with(".internal")
}

pub(super) fn denied_address(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.is_documentation()
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                // Unique local fc00::/7 and link-local fe80::/10.
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                // IPv4-mapped must obey the IPv4 policy; the deprecated
                // IPv4-compatible ::/96 space is denied wholesale.
                || (segments[..5] == [0, 0, 0, 0, 0] && segments[5] == 0)
                || (segments[..5] == [0, 0, 0, 0, 0]
                    && segments[5] == 0xffff
                    && denied_address(&IpAddr::V4(std::net::Ipv4Addr::new(
                        (segments[6] >> 8) as u8,
                        segments[6] as u8,
                        (segments[7] >> 8) as u8,
                        segments[7] as u8,
                    ))))
        }
    }
}

fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

#[derive(Clone)]
pub(super) struct Relay {
    port: u16,
    username: String,
    password: String,
    cancel: CancellationToken,
}

impl Relay {
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
    pub fn credentials(&self) -> (String, String) {
        (self.username.clone(), self.password.clone())
    }
    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

pub(super) async fn spawn_relay(allow_loopback: bool) -> AppResult<Relay> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| engine_error("The browser engine relay could not be started."))?;
    let port = listener.local_addr().map_err(|_| assets())?.port();
    let relay = Relay {
        port,
        username: Uuid::new_v4().to_string(),
        password: Uuid::new_v4().simple().to_string(),
        cancel: CancellationToken::new(),
    };
    let cancel = relay.cancel.clone();
    let expected = base64_credentials(&relay.username, &relay.password);
    tokio::spawn(async move {
        let lanes = Arc::new(tokio::sync::Semaphore::new(32));
        loop {
            let accept = tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                accepted = listener.accept() => accepted,
            };
            let (stream, _) = match accept {
                Ok(pair) => pair,
                Err(_) => continue,
            };
            let lane = match lanes.clone().try_acquire_owned() {
                Ok(lane) => lane,
                Err(_) => {
                    let mut stream = stream;
                    let _ = stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    continue;
                }
            };
            let expected = expected.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let _lane = lane;
                let _ = relay_connection(stream, &expected, allow_loopback, &cancel).await;
            });
        }
    });
    Ok(relay)
}

fn base64_credentials(username: &str, password: &str) -> Vec<u8> {
    let raw = format!("{}:{}", username, password);
    raw.as_bytes().to_vec()
}

const DENY: &[u8] = b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const CHALLENGE: &[u8] =
    b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"forma\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

async fn read_head(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if head.len() > 8192 {
            return None;
        }
        let count =
            match tokio::time::timeout(Duration::from_secs(20), stream.read(&mut chunk)).await {
                Ok(Ok(count)) => count,
                _ => return None,
            };
        if count == 0 {
            return None;
        }
        head.extend_from_slice(&chunk[..count]);
        if let Some(end) = find_head_end(&head) {
            let first = String::from_utf8_lossy(&head[..end]).to_string();
            return Some((first, head[end + 4..].to_vec()));
        }
    }
}

fn find_head_end(head: &[u8]) -> Option<usize> {
    head.windows(4).position(|w| w == b"\r\n\r\n")
}

/// CONNECT host:port | absolute-URI request. The relay resolves and connects;
/// the caller (engine) never resolves or connects by itself.
async fn relay_connection(
    mut client: TcpStream,
    expected: &[u8],
    allow_loopback: bool,
    cancel: &CancellationToken,
) -> AppResult<()> {
    let (first, rest) = match read_head(&mut client).await {
        Some(found) => found,
        None => return Ok(()),
    };
    let header_text = first.clone();
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let offered = header_text.split("\r\n").find_map(|line| {
        let lower = line.to_ascii_lowercase();
        if !lower.starts_with("proxy-authorization:") {
            return None;
        }
        // "Proxy-Authorization: Basic <token>" — compare only the token.
        line.split_once(' ')
            .and_then(|(_, rest)| rest.trim().split_once(' '))
            .map(|(_, token)| token.trim().to_string())
    });
    let authorized = matches!(&offered, Some(value)
        if constant_time_equal(value.as_bytes(), basic64(expected).as_bytes()));
    if !authorized {
        let _ = client.write_all(CHALLENGE).await;
        return Ok(());
    }
    let (host, port, upstream_target) = if method == "CONNECT" {
        let Some((host, port)) = target.rsplit_once(':') else {
            let _ = client.write_all(DENY).await;
            return Ok(());
        };
        (host.to_string(), port.parse::<u16>().unwrap_or(0), None)
    } else {
        let Ok(url) = Url::parse(&target) else {
            let _ = client.write_all(DENY).await;
            return Ok(());
        };
        let host = url.host_str().unwrap_or_default().to_string();
        let port = url.port_or_known_default().unwrap_or(0);
        (host, port, Some(url))
    };
    if denied_hostname(&host) {
        let _ = client.write_all(DENY).await;
        return Ok(());
    }
    let loopback_target = host
        .trim_end_matches('.')
        .parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false);
    let port_allowed = port == 80 || port == 443 || (allow_loopback && loopback_target);
    if !port_allowed || port == 0 {
        let _ = client.write_all(DENY).await;
        return Ok(());
    }
    if !allow_loopback && loopback_target {
        let _ = client.write_all(DENY).await;
        return Ok(());
    }
    let mut upstream = match resolve_and_connect(&host, port, allow_loopback).await {
        Ok(stream) => stream,
        Err(_) => {
            let _ = client
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            return Ok(());
        }
    };
    if method == "CONNECT" {
        let _ = client
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await;
    } else if let Some(url) = upstream_target {
        // Origin-form rewrite: the engine sends absolute-URI http requests.
        let origin = format!(
            "{}{}",
            url.path(),
            url.query().map(|q| format!("?{}", q)).unwrap_or_default()
        );
        let rewritten = format!(
            "{} {} HTTP/1.1\r\n",
            method,
            if origin.is_empty() { "/" } else { &origin }
        );
        let headers = header_text
            .split("\r\n")
            .skip(1)
            .filter(|line| {
                let lower = line.to_ascii_lowercase();
                !lower.starts_with("proxy-connection:")
                    && !lower.starts_with("proxy-authorization:")
            })
            .collect::<Vec<_>>()
            .join("\r\n");
        let head = format!("{}{}\r\n\r\n", rewritten, headers);
        upstream
            .write_all(head.as_bytes())
            .await
            .map_err(|_| network())?;
        upstream.write_all(&rest).await.map_err(|_| network())?;
    }
    let (mut client_read, mut client_write) = client.into_split();
    let (mut upstream_read, mut upstream_write) = upstream.into_split();
    let left = relay_half(&mut client_read, &mut upstream_write, cancel);
    let right = relay_half(&mut upstream_read, &mut client_write, cancel);
    let _ = tokio::join!(left, right);
    Ok(())
}

fn basic64(raw: &[u8]) -> String {
    // Minimal base64 for the constant-time authorization probe above.
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for block in raw.chunks(3) {
        let b = [
            block[0],
            block.get(1).copied().unwrap_or(0),
            block.get(2).copied().unwrap_or(0),
        ];
        let triple = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(triple >> 18) as usize & 63] as char);
        out.push(TABLE[(triple >> 12) as usize & 63] as char);
        out.push(if block.len() > 1 {
            TABLE[(triple >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if block.len() > 2 {
            TABLE[triple as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn network() -> AppError {
    AppError::new(
        "browser_engine_unavailable",
        "The Scrapling engine control channel failed.",
    )
}

async fn resolve_and_connect(host: &str, port: u16, allow_loopback: bool) -> AppResult<TcpStream> {
    let candidates = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| engine_error("The destination could not be resolved by the relay."))?;
    for address in candidates {
        if denied_address(&address.ip()) && !(allow_loopback && address.ip().is_loopback()) {
            continue;
        }
        if let Ok(Ok(connection)) =
            tokio::time::timeout(Duration::from_secs(15), TcpStream::connect(address)).await
        {
            return Ok(connection);
        }
    }
    Err(engine_error("The relay denied the destination."))
}

async fn relay_half(
    reader: &mut tokio::net::tcp::OwnedReadHalf,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    cancel: &CancellationToken,
) -> std::io::Result<()> {
    let mut chunk = [0u8; 65536];
    loop {
        let count = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(()),
            read = tokio::time::timeout(Duration::from_secs(120), reader.read(&mut chunk)) => {
                match read {
                    Ok(result) => result?,
                    Err(_) => return Ok(()),
                }
            }
        };
        if count == 0 {
            return Ok(());
        }
        writer.write_all(&chunk[..count]).await?;
    }
}

// ---------------------------------------------------------------------------
// Bundled browser verification (fail closed before any spawn).
// ---------------------------------------------------------------------------

pub(crate) fn browsers_root(app: &AppHandle) -> AppResult<PathBuf> {
    let root = app
        .path()
        .resource_dir()
        .map_err(|_| assets())?
        .join("managed-browsers");
    #[cfg(debug_assertions)]
    let root = if root.join("manifest.json").is_file() {
        root
    } else {
        option_env!("FORMA_MANAGED_BROWSERS_DIR")
            .map(PathBuf::from)
            .ok_or_else(assets)?
    };
    Ok(root)
}

/// Cheap availability probe used for engine advertisement; the full checksum
/// walk happens in [`verify_browsers`] before any process is started.
pub(crate) fn advertised(app: &AppHandle) -> bool {
    let Ok(root) = browsers_root(app) else {
        return false;
    };
    let Ok(manifest) = fs::read_to_string(root.join("manifest.json")) else {
        return false;
    };
    let Ok(manifest) = serde_json::from_str::<Value>(&manifest) else {
        return false;
    };
    manifest["schemaVersion"] == 1
        && manifest["platform"] == "darwin-arm64"
        && manifest["kind"] == CHROMIUM_KIND
        && manifest["chromium"]["revision"] == CHROMIUM_REVISION
}

fn resolve_inside(root: &Path, relative: &str) -> AppResult<PathBuf> {
    let relative = Path::new(relative);
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(assets());
    }
    let mut path = root.to_path_buf();
    for part in relative.components() {
        path.push(part);
        if fs::symlink_metadata(&path)
            .map_err(|_| assets())?
            .file_type()
            .is_symlink()
        {
            return Err(assets());
        }
    }
    Ok(path)
}

pub(crate) fn verify_browsers(root: &Path) -> AppResult<PathBuf> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err(assets());
    }
    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).map_err(|_| assets())?)
            .map_err(|_| assets())?;
    if manifest["schemaVersion"] != 1
        || manifest["platform"] != "darwin-arm64"
        || manifest["kind"] != CHROMIUM_KIND
        || manifest["chromium"]["revision"] != CHROMIUM_REVISION
    {
        return Err(assets());
    }
    let executable = manifest["chromium"]["executable"]
        .as_str()
        .ok_or_else(assets)?;
    let checksums: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(root.join("checksums.json")).map_err(|_| assets())?)
            .map_err(|_| assets())?;
    if checksums.is_empty() || checksums.len() > 4096 {
        return Err(assets());
    }
    for (relative, expected) in &checksums {
        let path = resolve_inside(root, relative)?;
        let mut file = fs::File::open(&path).map_err(|_| assets())?;
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash).map_err(|_| assets())?;
        if format!("{:x}", hash.finalize()) != *expected {
            return Err(assets());
        }
    }
    let binary = resolve_inside(root, executable)?;
    if !checksums.contains_key(executable) || !binary.is_file() {
        return Err(assets());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&binary)
            .map_err(|_| assets())?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(assets());
        }
    }
    Ok(binary)
}

// ---------------------------------------------------------------------------
// Read-only snapshot store rendered by the WebKit view via forma-snapshot://.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub id: String,
    pub url: Url,
    pub html: String,
}

#[derive(Default)]
pub(crate) struct SnapshotStore {
    pub session: Option<Uuid>,
    entries: VecDeque<Snapshot>,
}

impl SnapshotStore {
    pub fn reset(&mut self) {
        self.session = Some(Uuid::new_v4());
        self.entries.clear();
    }
    #[cfg(test)]
    pub(crate) fn newest_id(&self) -> Option<String> {
        self.entries.back().map(|entry| entry.id.clone())
    }
    pub fn insert(&mut self, url: Url, html: String) -> String {
        let id = Uuid::new_v4().simple().to_string();
        self.entries.push_back(Snapshot {
            id: id.clone(),
            url,
            html,
        });
        while self.entries.len() > SNAPSHOT_LIMIT {
            self.entries.pop_front();
        }
        id
    }
    pub fn find(&self, id: &str) -> Option<&Snapshot> {
        self.entries.iter().find(|entry| entry.id == id)
    }
    pub(crate) fn end(&mut self) {
        self.session = None;
        self.entries.clear();
    }
}

/// Relative links must resolve against the real origin so the navigation gate
/// can intercept them and route them back through the engine.
pub(super) fn shape_snapshot(url: &Url, html: &str, truncated: bool) -> String {
    let mut html = html.to_string();
    if html.len() > MAX_SNAPSHOT_HTML {
        html.truncate(MAX_SNAPSHOT_HTML);
    }
    let origin = format!("{}/", url.origin().ascii_serialization());
    let base = format!(
        "<base href=\"{}\" target=\"_self\"><meta name=\"referrer\" content=\"no-referrer\">",
        origin
    );
    let notice = if truncated {
        "<p class=\"forma-snapshot-note\">This snapshot was truncated because the page is very large.</p>"
    } else {
        ""
    };
    if let Some(position) = html.find("<head") {
        if let Some(close) = html[position..].find('>') {
            let at = position + close + 1;
            html.insert_str(at, &base);
            return html;
        }
    }
    format!(
        "<!doctype html><html><head>{}{}</head><body>{}</body></html>",
        base, notice, html
    )
}

pub(crate) fn home_snapshot(session: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\">\
<style>body{{margin:0;font-family:-apple-system,system-ui,sans-serif;background:#161618;color:#d5d5da;\
display:flex;min-height:100vh;align-items:center;justify-content:center}}\
main{{max-width:32rem;padding:2rem;line-height:1.5}}h1{{font-size:1.25rem;font-weight:600}}\
p{{color:#9a9aa2;font-size:.95rem}}</style></head><body><main><h1>Scrapling browser</h1>\
<p>This window shows read-only snapshots fetched by the Scrapling engine.</p>\
<p>Typing, signing in, and other page interactions are not supported yet. Links you click are \
fetched again by the engine.</p></main></body></html><!-- forma-snapshot:{} -->",
        session
    )
}

// ---------------------------------------------------------------------------
// Engine child controller: same pipe discipline as the Hermes controller, with
// engine-tuned bounds. No secret scrubbing is needed: none are ever sent.
// ---------------------------------------------------------------------------

struct EngineCall {
    frame: Value,
    reply: oneshot::Sender<AppResult<Value>>,
}

#[derive(Clone)]
pub(super) struct EngineController {
    tx: mpsc::Sender<EngineCall>,
    alive: Arc<AtomicBool>,
    reaped: Arc<AtomicBool>,
    stop: CancellationToken,
}

#[derive(Clone)]
pub(super) struct EngineRuntime {
    pub controller: EngineController,
    pub relay: Relay,
}

impl EngineController {
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Acquire) && !self.stop.is_cancelled()
    }
    pub async fn control(&self, mut frame: Value) -> AppResult<Value> {
        if !self.alive() {
            return Err(network());
        }
        frame["id"] = json!(Uuid::new_v4().to_string());
        let (reply, result) = oneshot::channel();
        self.tx
            .try_send(EngineCall { frame, reply })
            .map_err(|_| AppError::new("busy", "The browser engine is busy or stopped."))?;
        result.await.map_err(|_| network())?
    }
    pub async fn shutdown(&self) {
        self.stop.cancel();
        while !self.reaped.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

pub(super) async fn launch_engine(
    app: &AppHandle,
    executable: PathBuf,
    profile: PathBuf,
    relay: &Relay,
    fixture_root: Option<&Path>,
) -> AppResult<EngineRuntime> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err(assets());
    }
    let hermes = app
        .path()
        .resource_dir()
        .map_err(|_| assets())?
        .join("managed-hermes");
    #[cfg(debug_assertions)]
    let hermes = if hermes.join("manifest.json").is_file() {
        hermes
    } else {
        option_env!("FORMA_MANAGED_HERMES_DIR")
            .map(PathBuf::from)
            .ok_or_else(assets)?
    };
    let mut command = tokio::process::Command::new(hermes.join("python/bin/python3.12"));
    command
        .args(["-I", "-B"])
        .arg(hermes.join("controller/forma_runtime/engine.py"))
        .env_clear()
        .env("HOME", &profile)
        .env("TMPDIR", profile.join("tmp"))
        .env("PATH", "/usr/bin:/bin")
        .current_dir(&profile)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|_| network())?;
    let input = child.stdin.take().ok_or_else(network)?;
    let output = BufReader::new(child.stdout.take().ok_or_else(network)?);
    fs::create_dir_all(profile.join("tmp")).map_err(|_| crate::types::AppError::storage())?;
    let (username, password) = relay.credentials();
    let mut bootstrap = json!({
        "op": "bootstrap",
        "browsersExecutable": executable,
        "userDataDir": profile,
        "proxyServer": relay.endpoint(),
        "proxyUsername": username,
        "proxyPassword": password,
    });
    if let Some(root) = fixture_root {
        bootstrap["fixtureRoot"] = json!(root);
    }
    let (tx, rx) = mpsc::channel(4);
    let alive = Arc::new(AtomicBool::new(true));
    let reaped = Arc::new(AtomicBool::new(false));
    let stop = CancellationToken::new();
    let controller = EngineController {
        tx: tx.clone(),
        alive: alive.clone(),
        reaped: reaped.clone(),
        stop: stop.clone(),
    };
    tokio::spawn(engine_dispatch(
        child,
        input,
        output,
        rx,
        alive,
        reaped,
        stop.clone(),
    ));
    let runtime = EngineRuntime {
        controller: controller.clone(),
        relay: relay.clone(),
    };
    match controller.control(bootstrap).await {
        Ok(_) => Ok(runtime),
        Err(error) => {
            controller.shutdown().await;
            relay.stop();
            Err(error)
        }
    }
}

async fn engine_dispatch(
    mut child: Child,
    mut input: ChildStdin,
    mut output: BufReader<ChildStdout>,
    mut rx: mpsc::Receiver<EngineCall>,
    alive: Arc<AtomicBool>,
    reaped: Arc<AtomicBool>,
    stop: CancellationToken,
) {
    loop {
        let call = tokio::select! {
            biased;
            _ = stop.cancelled() => {
                let (reply, _) = oneshot::channel();
                EngineCall { frame: json!({"id": Uuid::new_v4().to_string(), "op": "close"}), reply }
            }
            call = rx.recv() => match call { Some(call) => call, None => break },
            _ = child.wait() => break,
        };
        let closing = call.frame["op"] == "close";
        if !closing && call.reply.is_closed() {
            continue;
        }
        let exchange = tokio::time::timeout(
            if closing {
                Duration::from_secs(10)
            } else {
                OPERATION_TIMEOUT
            },
            engine_exchange(&mut input, &mut output, call.frame.clone()),
        );
        let result = if closing {
            exchange.await.unwrap_or(Err(network()))
        } else {
            tokio::select! {
                biased;
                _ = stop.cancelled() => Err(network()),
                result = exchange => result.unwrap_or_else(|_| Err(network())),
            }
        };
        let broken = result.is_err();
        let _ = call.reply.send(result);
        if closing || broken {
            break;
        }
    }
    alive.store(false, Ordering::Release);
    drop(input);
    if tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .is_err()
    {
        #[cfg(unix)]
        if let Some(pid) = child.id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    reaped.store(true, Ordering::Release);
    while let Ok(call) = rx.try_recv() {
        let _ = call.reply.send(Err(network()));
    }
}

async fn engine_exchange(
    input: &mut ChildStdin,
    output: &mut BufReader<ChildStdout>,
    frame: Value,
) -> AppResult<Value> {
    let id = frame["id"].as_str().ok_or_else(network)?.to_owned();
    let mut bytes = serde_json::to_vec(&frame).map_err(|_| network())?;
    if bytes.len() + 1 > MAX_REQUEST {
        return Err(network());
    }
    bytes.push(b'\n');
    input.write_all(&bytes).await.map_err(|_| network())?;
    input.flush().await.map_err(|_| network())?;
    let mut response = Vec::new();
    loop {
        let buffer = output.fill_buf().await.map_err(|_| network())?;
        if buffer.is_empty() {
            return Err(network());
        }
        let end = buffer.iter().position(|b| *b == b'\n').map(|n| n + 1);
        let count = end.unwrap_or(buffer.len());
        if response.len() + count > MAX_RESPONSE {
            return Err(network());
        }
        response.extend_from_slice(&buffer[..count]);
        output.consume(count);
        if end.is_some() {
            break;
        }
    }
    if response.len() > MAX_RESPONSE {
        return Err(network());
    }
    let envelope: Value = serde_json::from_slice(&response).map_err(|_| network())?;
    let object = envelope.as_object().ok_or_else(network)?;
    if object.len() != 3
        || !object.contains_key("id")
        || !object.contains_key("ok")
        || (!object.contains_key("result") && !object.contains_key("error"))
        || envelope["id"] != id
    {
        return Err(network());
    }
    if envelope["ok"] != true {
        // The wire carries child-authored text; surface only reviewed static
        // messages keyed by the child's error code.
        let message: &'static str = match envelope["error"]["code"].as_str().unwrap_or("") {
            "destination_denied" | "destination_frozen" => {
                "That destination is not allowed through the browser engine."
            }
            "engine_navigation_failed" => "The page could not be fetched by the Scrapling engine.",
            "engine_fetch_failed" => "The address could not be fetched by the Scrapling engine.",
            "browser_unavailable" | "profile_unavailable" => {
                "The browser engine could not be prepared."
            }
            "not_bootstrapped" | "already_bootstrapped" | "relay_required" => {
                "The browser engine is not in a valid state."
            }
            _ => "The Scrapling engine could not complete this operation.",
        };
        return Err(AppError::new("browser_engine_failed", message));
    }
    envelope.get("result").cloned().ok_or_else(network)
}

/// Snapshot lookup backing the forma-snapshot protocol and navigation gate.
pub(crate) type SharedSnapshots = Arc<Mutex<SnapshotStore>>;

pub(crate) fn snapshot_response(
    store: &SharedSnapshots,
    session: &str,
    id: &str,
) -> Option<(String, &'static str)> {
    let store = store.lock().ok()?;
    let store = &*store;
    if store.session?.to_string().as_str() != session {
        return None;
    }
    if id == "home" {
        return Some((home_snapshot(session), "text/html; charset=utf-8"));
    }
    let entry = store.find(id)?;
    Some((entry.html.clone(), "text/html; charset=utf-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostnames_fail_closed() {
        for host in [
            "higgsfield.ai",
            "cdn.HIGGSFIELD.io",
            "a.higgsfield.com.",
            "localhost",
            "x.localhost",
            "printer.local",
            "portal.internal",
        ] {
            assert!(denied_hostname(host), "{host}");
        }
        assert!(!denied_hostname("example.com"));
        // Name-based denial stays conservative: any host containing the
        // frozen brand is denied, mirroring the WebKit gate.
        assert!(denied_hostname("example-higgsfield-free.test"));
    }

    #[test]
    fn addresses_fail_closed() {
        for text in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.2",
            "172.16.0.9",
            "169.254.169.254",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "::1",
            "fe80::1",
            "fd12:3456::1",
            "fc00::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
        ] {
            let ip: IpAddr = text.parse().unwrap();
            assert!(denied_address(&ip), "{text}");
        }
        for text in ["93.184.216.34", "2606:2800:220:1:248:1893:25c8:1946"] {
            let ip: IpAddr = text.parse().unwrap();
            assert!(!denied_address(&ip), "{text}");
        }
    }

    #[test]
    fn snapshots_are_bounded_and_session_scoped() {
        let mut store = SnapshotStore::default();
        store.reset();
        let session = store.session.unwrap().to_string();
        let url = Url::parse("https://example.com/page").unwrap();
        let first = store.insert(url.clone(), "<html>a</html>".into());
        for index in 0..SNAPSHOT_LIMIT {
            store.insert(url.clone(), format!("<html>{}</html>", index));
        }
        assert!(store.find(&first).is_none());
        assert!(store.find(&store.newest_id().unwrap()).is_some());
        assert!(snapshot_response(
            &Arc::new(Mutex::new(store)),
            "00000000-0000-0000-0000-000000000000",
            "home"
        )
        .is_none());
        let _ = session;
    }

    #[test]
    fn shaping_injects_the_real_origin() {
        let url = Url::parse("https://example.com/dir/page?x=1").unwrap();
        let shaped = shape_snapshot(
            &url,
            "<html><head><title>t</title></head><body>hi</body></html>",
            false,
        );
        assert!(shaped.contains("<base href=\"https://example.com/\""));
        assert!(shaped.contains("<title>t</title>"));
        let headless = shape_snapshot(&url, "<p>fragment</p>", true);
        assert!(
            headless.starts_with("<!doctype html><html><head><base href=\"https://example.com/\"")
        );
        assert!(headless.contains("forma-snapshot-note"));
        let big = shape_snapshot(&url, &"<p>x</p>".repeat(MAX_SNAPSHOT_HTML), false);
        assert!(big.len() < MAX_SNAPSHOT_HTML + 4096);
    }

    #[tokio::test]
    async fn relay_denies_unauthenticated_and_local_targets() {
        let relay = spawn_relay(true).await.unwrap();
        // Unauthenticated CONNECT is challenged.
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", relay.port))
            .await
            .unwrap();
        stream
            .write_all(b"CONNECT 127.0.0.1:9 HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut head = vec![0u8; 512];
        let count = stream.read(&mut head).await.unwrap();
        assert!(
            head[..count].starts_with(b"HTTP/1.1 407"),
            "{}",
            String::from_utf8_lossy(&head[..count])
        );
        relay.stop();
    }

    #[tokio::test]
    async fn relay_relay_authorized_loopback_when_test_mode() {
        let relay = spawn_relay(true).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            sock.write_all(b"pong").await.unwrap();
        });
        let (username, password) = relay.credentials();
        let auth = basic64(&base64_credentials(&username, &password));
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", relay.port))
            .await
            .unwrap();
        let request = format!(
            "CONNECT {} HTTP/1.1\r\nHost: {}\r\nProxy-Authorization: Basic {}\r\n\r\n",
            target, target, auth
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut head = vec![0u8; 512];
        let count = stream.read(&mut head).await.unwrap();
        assert!(
            head[..count].starts_with(b"HTTP/1.1 200"),
            "{}",
            String::from_utf8_lossy(&head[..count])
        );
        stream.write_all(b"ping").await.unwrap();
        let mut pong = [0u8; 4];
        stream.read_exact(&mut pong).await.unwrap();
        assert_eq!(&pong, b"pong");
        relay.stop();
    }

    #[test]
    fn basic64_matches_the_standard_alphabet() {
        assert_eq!(basic64(b"user:pass"), "dXNlcjpwYXNz");
        assert_eq!(basic64(b"a"), "YQ==");
        assert_eq!(basic64(b"ab"), "YWI=");
        assert_eq!(basic64(b"abc"), "YWJj");
    }
}
