//! App-owned browsing, deliberately separate from the privileged renderer.
//!
//! WebKit on macOS 14+ is the only available engine. Chromium requests fail
//! closed before binary detection, profile access, pipe setup or process spawn:
//! post-launch CDP filters cannot prove startup/restoration/background denial.

mod chromium;
pub(crate) mod engine;
#[cfg(target_os = "macos")]
mod macos;
mod profile;

use crate::types::{AppError, AppResult};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

pub const ENGINE_SCRAPLING: &str = "scrapling";
const ENGINE_NAVIGATE_FAILED: &str = "The page could not be fetched by the Scrapling engine.";

pub const LABEL: &str = "owned-browser";
const MAX_URL_BYTES: usize = 4096;
const UNAVAILABLE: &str = "Owned browsing requires macOS 14 or later with an isolated WebKit profile and native content blocking.";
pub(super) const GATE_ERROR: &str =
    "The isolated browser could not be secured. No external page was opened.";

/// Every security gate funnels through here: one line per failure lands in
/// `<app data>/browser-gate.log` (site + error code + timestamp) so a live
/// failure is diagnosable without GUI automation. The user-visible message
/// stays GATE_ERROR; only the code/log differ per site.
pub(super) fn gate(app: Option<&AppHandle>, site: &str, code: &str) -> AppError {
    if let Some(app) = app {
        if let Ok(dir) = app.path().app_data_dir() {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("browser-gate.log"))
            {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let _ = writeln!(file, "{now} site={site} code={code}");
            }
        }
    }
    AppError::new(
        match code {
            "browser_profile_unavailable" => "browser_profile_unavailable",
            _ => "browser_unavailable",
        },
        GATE_ERROR,
    )
}

/// Requested engine backend. Wire-level identifier the frontend passes when
/// calling `browser_open`; missing/None defaults to `webkit`.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserEngine {
    Webkit,
    Chromium,
    Scrapling,
}

/// Detected engine, exposed to the frontend so it can render a *truthful*
/// toggle (label matches the actual host binary — never "Chromium" for a
/// WebKit session).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AvailableEngine {
    pub id: &'static str,
    pub label: &'static str,
}

/// Compute the engines available on this host. Ordered: default first.
///
/// Never spawns anything or inspects Chromium installations/profiles. Windows
/// and Linux remain unavailable rather than advertising an unsupported runtime.
pub fn browser_engines(app: Option<&AppHandle>) -> Vec<AvailableEngine> {
    let mut engines: Vec<AvailableEngine> = Vec::new();
    // Founder directive 2026-09-13: the integrated browser is Scrapling. It is
    // advertised first (and used as the default) only when the sealed managed
    // browser resource is present; otherwise WebKit remains, visibly.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if app.is_some_and(engine::advertised) {
        engines.push(AvailableEngine {
            id: ENGINE_SCRAPLING,
            label: "Scrapling (Chromium snapshots)",
        });
    }
    #[cfg(target_os = "macos")]
    if macos::supported() {
        engines.push(AvailableEngine {
            id: "webkit",
            label: "WebKit",
        });
    }
    engines
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserService {
    Home,
    Gmail,
    GoogleCalendar,
    Custom,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserPhase {
    Closed,
    Opening,
    Open,
    Error,
    Unavailable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedBrowserStatus {
    revision: u64,
    available: bool,
    phase: BrowserPhase,
    /// Engine actually in use for this session — "webkit", "chromium", or
    /// "unavailable" when no engine has been opened.
    engine: &'static str,
    /// Every engine the host supports right now (empty on unsupported hosts).
    available_engines: Vec<AvailableEngine>,
    chromium_unavailable_reason: &'static str,
    persistent: bool,
    profile_id: Option<String>,
    url: Option<String>,
    service: Option<BrowserService>,
    error: Option<&'static str>,
    automation_ready: bool,
    /// False for Scrapling snapshot sessions: the window shows fetched,
    /// read-only pages; typing and sign-in are not wired to the engine.
    interactive: bool,
    /// True while the Scrapling engine is fetching a page in the background.
    navigating: bool,
}

pub(super) struct Session {
    status: OwnedBrowserStatus,
    generation: u64,
    secured: bool,
}

#[derive(Clone)]
pub struct BrowserState {
    inner: Arc<Mutex<Session>>,
    directory: Option<PathBuf>,
    app: Option<AppHandle>,
    allow_loopback: bool,
    engine: Arc<Mutex<Option<engine::EngineRuntime>>>,
    pub(crate) snapshots: engine::SharedSnapshots,
}

impl BrowserState {
    pub fn new(directory: Option<PathBuf>, qa_override: bool, app: Option<&AppHandle>) -> Self {
        let engines = browser_engines(app);
        let available = !engines.is_empty();
        // Default engine identifier when nothing is running yet — matches the
        // engine `browser_open` will pick when the caller omits `engine`.
        let default_engine: &'static str = engines.first().map(|e| e.id).unwrap_or("unavailable");
        Self {
            inner: Arc::new(Mutex::new(Session {
                status: OwnedBrowserStatus {
                    revision: 0,
                    available,
                    phase: if available {
                        BrowserPhase::Closed
                    } else {
                        BrowserPhase::Unavailable
                    },
                    engine: default_engine,
                    available_engines: engines,
                    chromium_unavailable_reason: chromium::UNAVAILABLE_REASON,
                    persistent: false,
                    profile_id: None,
                    url: None,
                    service: None,
                    error: if available { None } else { Some(UNAVAILABLE) },
                    automation_ready: false,
                    interactive: false,
                    navigating: false,
                },
                generation: 0,
                secured: false,
            })),
            directory,
            app: app.cloned(),
            allow_loopback: cfg!(debug_assertions) && qa_override,
            engine: Arc::new(Mutex::new(None)),
            snapshots: Arc::new(Mutex::new(engine::SnapshotStore::default())),
        }
    }

    pub fn with_app(mut self, app: AppHandle) -> Self {
        self.app = Some(app);
        self
    }

    /// Engine and relay teardown is bounded; snapshots die with the session.
    pub async fn stop_engine(&self) {
        let runtime = self.engine.lock().ok().and_then(|mut slot| slot.take());
        if let Some(runtime) = runtime {
            runtime.controller.shutdown().await;
            runtime.relay.stop();
        }
        if let Ok(mut store) = self.snapshots.lock() {
            store.end();
        }
    }

    fn emit(&self) {
        if let (Some(app), Ok(status)) = (&self.app, self.status()) {
            // Never broadcast browser state (including visited origins) to remote content.
            let _ = app.emit_to(
                tauri::EventTarget::webview_window("main"),
                "owned-browser:state",
                status,
            );
        }
    }

    pub fn shutdown(&self) {
        if let Ok(generation) = self.lock().map(|s| s.generation) {
            self.closed(generation, false);
        }
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, Session>> {
        self.inner
            .lock()
            .map_err(|_| AppError::new("browser_unavailable", GATE_ERROR))
    }

    fn status(&self) -> AppResult<OwnedBrowserStatus> {
        let mut session = self.lock()?;
        session.status.revision = session.status.revision.saturating_add(1);
        Ok(session.status.clone())
    }

    fn fail(&self, generation: u64) {
        if let Ok(mut session) = self.lock() {
            if session.generation == generation {
                session.secured = false;
                session.status.phase = BrowserPhase::Error;
                session.status.error = Some(GATE_ERROR);
                session.status.url = None;
                session.status.navigating = false;
            }
        }
        self.emit();
    }

    fn expire(&self, generation: u64) {
        // Timeout and completion can race across threads: check+transition under one lock.
        if let Ok(mut s) = self.lock() {
            if s.generation == generation && s.status.phase == BrowserPhase::Opening {
                s.secured = false;
                s.status.phase = BrowserPhase::Error;
                s.status.error = Some(GATE_ERROR);
                s.status.url = None;
                s.status.navigating = false;
            }
        }
        self.emit();
    }

    fn current(&self, generation: u64) -> bool {
        self.lock()
            .map(|s| s.generation == generation && s.status.phase == BrowserPhase::Opening)
            .unwrap_or(false)
    }

    fn validate_url(&self, raw: &str) -> AppResult<Url> {
        validate_url_with_callback(raw, self.allow_loopback, |url| {
            self.app
                .as_ref()
                .and_then(|app| app.try_state::<crate::connectors::ConnectorState>())
                .map(|connectors| connectors.allows_pending_callback(url))
                .unwrap_or(false)
        })
    }

    fn permits(&self, generation: u64, url: &Url) -> bool {
        // Do not nest the browser and connector mutexes. The callback listener
        // independently validates state/expiry again before accepting a response.
        if url.as_str() == "about:blank" {
            return self
                .lock()
                .map(|s| s.generation == generation)
                .unwrap_or(false);
        }
        if url.scheme() == engine::SNAPSHOT_SCHEME {
            return self.permits_snapshot(generation, url);
        }
        let session = self
            .lock()
            .map(|s| (s.generation == generation && s.secured, s.status.engine))
            .unwrap_or((false, "unavailable"));
        if session.1 == ENGINE_SCRAPLING {
            // Snapshot link clicks never load directly: they are routed back
            // through the Scrapling engine and re-rendered as a new snapshot.
            if session.0 && matches!(url.scheme(), "https" | "http") {
                if let Ok(target) = self.validate_url(url.as_str()) {
                    if let Some(app) = self.app.clone() {
                        let state = self.clone();
                        tauri::async_runtime::spawn(async move {
                            engine_navigate(&app, &state, target).await;
                        });
                    }
                }
            }
            return false;
        }
        let allowed = self.validate_url(url.as_str()).is_ok();
        self.lock()
            .map(|s| s.generation == generation && (s.secured && allowed))
            .unwrap_or(false)
    }

    fn permits_snapshot(&self, generation: u64, url: &Url) -> bool {
        let engine_live = self
            .lock()
            .map(|s| s.generation == generation && s.secured && s.status.engine == ENGINE_SCRAPLING)
            .unwrap_or(false);
        if !engine_live {
            return false;
        }
        let Some(session) = url.host_str() else {
            return false;
        };
        let path = url.path().trim_start_matches('/');
        if path.contains('/') || path.is_empty() {
            return false;
        }
        let store = self.snapshots.lock();
        let Ok(store) = store else { return false };
        store.session.map(|id| id.to_string()).as_deref() == Some(session)
            && (path == "home" || store.find(path).is_some())
    }

    fn closed(&self, generation: u64, preserve_error: bool) {
        let engine_was_live = self
            .lock()
            .map(|s| {
                s.generation == generation
                    && s.status.engine == ENGINE_SCRAPLING
                    && self
                        .engine
                        .lock()
                        .map(|slot| slot.is_some())
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        if let Ok(mut s) = self.lock() {
            if s.generation == generation {
                s.generation = s.generation.wrapping_add(1);
                s.secured = false;
                if !preserve_error || s.status.phase != BrowserPhase::Error {
                    s.status.phase = BrowserPhase::Closed;
                    s.status.error = None;
                }
                s.status.url = None;
                s.status.service = None;
                s.status.navigating = false;
            }
        }
        if engine_was_live {
            let state = self.clone();
            tauri::async_runtime::spawn(async move {
                state.stop_engine().await;
            });
        }
        self.emit();
    }
}

fn invalid_url() -> AppError {
    AppError::new("browser_url_rejected", "Use an HTTPS address without embedded credentials. Restricted destinations and non-web protocols are not allowed.")
}

/// Conservative name-based denial, including alternate TLDs and subdomains.
/// Kept broader than a list of currently known domains; never probes a host.
fn denied_host(host: &str) -> bool {
    host.to_ascii_lowercase().contains("higgsfield")
}

fn validate_url(raw: &str, allow_loopback: bool) -> AppResult<Url> {
    if raw.is_empty()
        || raw.len() > MAX_URL_BYTES
        || raw.chars().any(|c| c.is_control() || c.is_whitespace())
        || raw.contains('\\')
    {
        return Err(invalid_url());
    }
    let url = Url::parse(raw).map_err(|_| invalid_url())?;
    let host = url.host_str().ok_or_else(invalid_url)?;
    let loopback = host
        .trim_matches(['[', ']'])
        .parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or_else(|_| host.trim_end_matches('.').eq_ignore_ascii_case("localhost"));
    if !url.username().is_empty()
        || url.password().is_some()
        || denied_host(host)
        || (loopback && !allow_loopback)
        || host
            .trim_end_matches('.')
            .eq_ignore_ascii_case("tauri.localhost")
        || !(url.scheme() == "https" || (url.scheme() == "http" && allow_loopback && loopback))
    {
        return Err(invalid_url());
    }
    // Empty userinfo is still an embedded-credential syntax and must not normalize away.
    let authority = raw
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""));
    if authority.map(|s| s.contains('@')).unwrap_or(true) {
        return Err(invalid_url());
    }
    Ok(url)
}

/// This exception grants no general production loopback access. The host-owned
/// connector matcher must prove an exact, live attempt; standard URL hygiene and
/// restricted-destination checks still apply before it is even consulted.
fn validate_url_with_callback(
    raw: &str,
    allow_loopback: bool,
    pending: impl FnOnce(&Url) -> bool,
) -> AppResult<Url> {
    if let Ok(url) = validate_url(raw, allow_loopback) {
        return Ok(url);
    }
    let url = validate_url(raw, true)?;
    if url.scheme() == "http" && url.host_str() == Some("127.0.0.1") && pending(&url) {
        Ok(url)
    } else {
        Err(invalid_url())
    }
}

fn service_url(service: BrowserService, url: Option<&str>, qa: bool) -> AppResult<Url> {
    match (service, url) {
        (BrowserService::Home, None) => Url::parse("about:blank").map_err(|_| invalid_url()),
        (BrowserService::Gmail, None) => validate_url("https://mail.google.com/", qa),
        (BrowserService::GoogleCalendar, None) => validate_url("https://calendar.google.com/", qa),
        (BrowserService::Custom, Some(raw)) => validate_url(raw, qa),
        _ => Err(invalid_url()),
    }
}

/// Origin only: paths may also contain sign-in codes, document IDs, or API keys.
/// Never return page titles, raw navigation errors, query strings or fragments.
fn public_url(url: &Url) -> Option<String> {
    if url.as_str() == "about:blank" {
        return Some("about:blank".into());
    }
    if validate_url(url.as_str(), true).is_err() {
        return None;
    }
    Some(format!("{}/", url.origin().ascii_serialization()))
}

/// Native resource rules, installed before creating any remotely navigable view.
/// No resource-type restriction: documents, frames, fetches, scripts, media, etc.
fn content_rules() -> String {
    serde_json::json!([{
        "trigger": {"url-filter": ".*higgsfield.*", "url-filter-is-case-sensitive": false},
        "action": {"type": "block"}
    }])
    .to_string()
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum History {
    Back,
    Forward,
    Reload,
}

enum Action {
    Status,
    Open(BrowserService, Option<String>, BrowserEngine),
    Navigate(String),
    Close,
    History(History),
}

/// Resolve the engine requested by the caller against what the host actually
/// supports. The default is the first advertised engine — Scrapling when its
/// sealed resources verify (founder directive 2026-09-13), WebKit otherwise.
/// The fallback is visible in the engine list, never a silent swap; explicit
/// Chromium requests remain unavailable.
fn resolve_engine(
    request: Option<BrowserEngine>,
    advertised: &[AvailableEngine],
) -> AppResult<BrowserEngine> {
    let engines = advertised;
    let default = match engines.first() {
        Some(engine) if engine.id == ENGINE_SCRAPLING => BrowserEngine::Scrapling,
        _ => BrowserEngine::Webkit,
    };
    let want = request.unwrap_or(default);
    if want == BrowserEngine::Chromium {
        chromium::require_available()?;
    }
    let wanted_id = match want {
        BrowserEngine::Webkit => "webkit",
        BrowserEngine::Chromium => "chromium",
        BrowserEngine::Scrapling => ENGINE_SCRAPLING,
    };
    if engines.iter().any(|e| e.id == wanted_id) {
        Ok(want)
    } else {
        Err(AppError::new("browser_unavailable", GATE_ERROR))
    }
}

async fn start_engine(app: &AppHandle, state: &BrowserState) -> AppResult<()> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err(AppError::new(
            "browser_unavailable",
            "The Scrapling engine is not packaged for this platform.",
        ));
    }
    let root = engine::browsers_root(app)?;
    let executable = tokio::task::spawn_blocking(move || engine::verify_browsers(&root))
        .await
        .map_err(|_| AppError::new("browser_unavailable", GATE_ERROR))??;
    let profile = state
        .directory
        .as_ref()
        .ok_or_else(|| AppError::new("browser_profile_unavailable", GATE_ERROR))?
        .join("managed-browser-engine");
    std::fs::create_dir_all(&profile).map_err(|_| AppError::storage())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| AppError::storage())?;
    }
    let relay = engine::spawn_relay(state.allow_loopback).await?;
    let fixture_root = state
        .allow_loopback
        .then(|| std::env::var_os("FORMA_ENGINE_FIXTURE_ROOT").map(PathBuf::from))
        .flatten();
    let runtime =
        engine::launch_engine(app, executable, profile, &relay, fixture_root.as_deref()).await?;
    state
        .engine
        .lock()
        .map_err(|_| AppError::new("browser_unavailable", GATE_ERROR))?
        .replace(runtime);
    Ok(())
}

/// Fetch `url` through the engine and re-render the window from the snapshot.
/// Failures leave the previous snapshot visible with an honest status error.
async fn engine_navigate(app: &AppHandle, state: &BrowserState, url: Url) {
    let generation = state.lock().map(|s| s.generation).unwrap_or(u64::MAX);
    {
        let Ok(mut s) = state.lock() else { return };
        if s.generation != generation || s.status.engine != ENGINE_SCRAPLING {
            return;
        }
        s.status.navigating = true;
        s.status.error = None;
    }
    state.emit();
    let controller = state
        .engine
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|runtime| runtime.controller.clone()));
    let outcome = match controller {
        Some(controller) => {
            controller
                .control(serde_json::json!({"op": "navigate", "url": url.as_str()}))
                .await
        }
        None => Err(AppError::new(
            "browser_engine_failed",
            ENGINE_NAVIGATE_FAILED,
        )),
    };
    let applied = match outcome {
        Ok(value) => {
            let final_url = value["url"]
                .as_str()
                .and_then(|text| Url::parse(text).ok())
                .unwrap_or_else(|| url.clone());
            let truncated = value["truncated"] == true;
            let shaped = engine::shape_snapshot(
                &final_url,
                value["html"].as_str().unwrap_or_default(),
                truncated,
            );
            let stored = state
                .snapshots
                .lock()
                .ok()
                .and_then(|mut store| Some(store.insert(final_url.clone(), shaped)));
            stored
                .zip(
                    state
                        .snapshots
                        .lock()
                        .ok()
                        .and_then(|store| store.session.map(|id| id.to_string())),
                )
                .and_then(|(id, session)| {
                    Url::parse(&format!("{}://{}/{}", engine::SNAPSHOT_SCHEME, session, id)).ok()
                })
                .and_then(|target| {
                    let window = app.get_webview_window(LABEL)?;
                    window.navigate(target).ok()?;
                    Some(final_url)
                })
        }
        Err(_) => None,
    };
    if let Ok(mut s) = state.lock() {
        if s.generation == generation {
            s.status.navigating = false;
            match applied {
                Some(final_url) => {
                    s.status.url = public_url(&final_url);
                    s.status.service = Some(BrowserService::Custom);
                }
                None => s.status.error = Some(ENGINE_NAVIGATE_FAILED),
            }
        }
    }
    state.emit();
}

async fn open_scrapling(
    app: AppHandle,
    state: BrowserState,
    service: BrowserService,
    raw: Option<String>,
) -> AppResult<OwnedBrowserStatus> {
    let url = match (service, raw.as_deref()) {
        (BrowserService::Custom, Some(raw)) => state.validate_url(raw)?,
        _ => service_url(service, raw.as_deref(), state.allow_loopback)?,
    };
    let phase = state.status()?.phase;
    if phase == BrowserPhase::Opening {
        return Err(AppError::new(
            "browser_busy",
            "The browser is still securing its profile. Try again shortly.",
        ));
    }
    let live = state
        .lock()
        .map(|s| s.secured && s.status.engine == ENGINE_SCRAPLING)
        .unwrap_or(false)
        && app.get_webview_window(LABEL).is_some();
    if live {
        if service != BrowserService::Home {
            engine_navigate(&app, &state, url).await;
        }
        if let Some(window) = app.get_webview_window(LABEL) {
            window
                .show()
                .and_then(|_| window.set_focus())
                .map_err(|_| {
                    AppError::new(
                        "browser_focus_failed",
                        "The browser is open, but its window could not be brought forward.",
                    )
                })?;
        }
        state.emit();
        return state.status();
    }
    let generation = {
        let mut s = state.lock()?;
        s.generation = s.generation.wrapping_add(1);
        s.secured = false;
        s.status.phase = BrowserPhase::Opening;
        s.status.error = None;
        s.status.url = None;
        s.status.service = Some(service);
        s.status.engine = ENGINE_SCRAPLING;
        s.status.navigating = url.as_str() != "about:blank";
        s.status.interactive = false;
        s.generation
    };
    if let Ok(mut store) = state.snapshots.lock() {
        store.reset();
    }
    let session = state
        .snapshots
        .lock()
        .ok()
        .and_then(|store| store.session.map(|id| id.to_string()))
        .ok_or_else(|| AppError::new("browser_unavailable", GATE_ERROR))?;
    let home = Url::parse(&format!("{}://{}/home", engine::SNAPSHOT_SCHEME, session))
        .map_err(|_| AppError::new("browser_unavailable", GATE_ERROR))?;
    let timeout_state = state.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(75)).await;
        timeout_state.expire(generation);
    });
    #[cfg(target_os = "macos")]
    {
        if macos::open(&app, &state, generation, home, true).is_err() {
            state.fail(generation);
            return state.status();
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        state.fail(generation);
        return Err(AppError::new("browser_unavailable", UNAVAILABLE));
    }
    if let Err(error) = start_engine(&app, &state).await {
        state.fail(generation);
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = window.destroy();
        }
        state.closed(generation, true);
        return Err(error);
    }
    if url.as_str() != "about:blank" {
        engine_navigate(&app, &state, url).await;
    } else if let Ok(mut s) = state.lock() {
        if s.generation == generation && s.status.phase == BrowserPhase::Opening {
            s.status.navigating = false;
        }
    }
    state.emit();
    state.status()
}

async fn perform(
    app: AppHandle,
    state: BrowserState,
    action: Action,
) -> AppResult<OwnedBrowserStatus> {
    // Await the native history result without blocking the main thread; a
    // successful dispatcher enqueue is not proof that navigation was allowed.
    if let Action::History(direction) = &action {
        let window = secured_window(&app, &state)?;
        // Reload re-fetches through the engine: a stored snapshot is never
        // silently re-served as if it were fresh.
        if *direction == History::Reload
            && state
                .lock()
                .map(|s| s.status.engine == ENGINE_SCRAPLING && s.secured)
                .unwrap_or(false)
        {
            let target = window
                .url()
                .ok()
                .and_then(|url| Url::parse(url.as_str()).ok());
            let snapshot = target
                .as_ref()
                .filter(|url| url.scheme() == engine::SNAPSHOT_SCHEME)
                .and_then(|url| {
                    let id = url.path().trim_start_matches('/');
                    state
                        .snapshots
                        .lock()
                        .ok()
                        .and_then(|store| store.find(id).cloned())
                });
            if let Some(snapshot) = snapshot {
                engine_navigate(&app, &state, snapshot.url).await;
                state.emit();
                return state.status();
            }
        }
        #[cfg(target_os = "macos")]
        macos::history(&window, state.clone(), *direction).await?;
        #[cfg(not(target_os = "macos"))]
        return Err(AppError::new("browser_unavailable", UNAVAILABLE));
        state.emit();
        return state.status();
    }
    if let Action::Open(service, raw, BrowserEngine::Scrapling) = &action {
        return open_scrapling(app.clone(), state.clone(), *service, raw.clone()).await;
    }
    if let Action::Navigate(raw) = &action {
        if state
            .lock()
            .map(|s| s.status.engine == ENGINE_SCRAPLING && s.secured)
            .unwrap_or(false)
        {
            let url = state.validate_url(raw)?;
            engine_navigate(&app, &state, url).await;
            state.emit();
            return state.status();
        }
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = perform_on_main(&handle, &state, action);
        let _ = tx.send(result);
    })
    .map_err(|_| AppError::new("browser_unavailable", GATE_ERROR))?;
    rx.await
        .map_err(|_| AppError::new("browser_unavailable", GATE_ERROR))?
}

fn perform_on_main(
    app: &AppHandle,
    state: &BrowserState,
    action: Action,
) -> AppResult<OwnedBrowserStatus> {
    if !state.status()?.available {
        return if matches!(action, Action::Status) {
            state.status()
        } else {
            Err(AppError::new("browser_unavailable", UNAVAILABLE))
        };
    }
    let changed = !matches!(action, Action::Status);
    match action {
        Action::Status => {
            if state.lock()?.secured {
                let engine_session = state.lock()?.status.engine.eq(ENGINE_SCRAPLING);
                if let Some(window) = app.get_webview_window(LABEL) {
                    if !engine_session {
                        // url() is native and handles same-document/history URL changes, too.
                        let url = window.url().ok().and_then(|u| public_url(&u));
                        state.lock()?.status.url = url;
                    }
                } else {
                    let generation = state.lock()?.generation;
                    state.closed(generation, false);
                }
            }
        }
        Action::Close => {
            let generation = state.lock()?.generation;
            if let Some(window) = app.get_webview_window(LABEL) {
                window.destroy().map_err(|_| {
                    AppError::new(
                        "browser_close_failed",
                        "The browser window could not be closed.",
                    )
                })?;
            }
            state.closed(generation, false);
        }
        Action::Open(service, raw, engine) => {
            let url = match (service, raw.as_deref()) {
                (BrowserService::Custom, Some(raw)) => state.validate_url(raw)?,
                _ => service_url(service, raw.as_deref(), state.allow_loopback)?,
            };
            let phase = state.status()?.phase;
            if phase == BrowserPhase::Opening {
                return Err(AppError::new(
                    "browser_busy",
                    "The browser is still securing its profile. Try again shortly.",
                ));
            }
            match engine {
                BrowserEngine::Webkit => {
                    if let Some(window) = app.get_webview_window(LABEL) {
                        if !state.lock()?.secured {
                            return Err(AppError::new("browser_unavailable", GATE_ERROR));
                        }
                        // Reopening Home focuses an existing session; never interrupts a sign-in.
                        if service != BrowserService::Home {
                            window.navigate(url).map_err(|_| {
                                AppError::new(
                                    "browser_navigation_failed",
                                    "The browser could not navigate.",
                                )
                            })?;
                            state.lock()?.status.service = Some(service);
                        }
                        window.show().and_then(|_| window.set_focus()).map_err(|_| {
                            AppError::new("browser_focus_failed", "The browser is open, but its window could not be brought forward.")
                        })?;
                        state.lock()?.status.error = None;
                    } else {
                        let generation = {
                            let mut s = state.lock()?;
                            s.generation = s.generation.wrapping_add(1);
                            s.secured = false;
                            s.status.phase = BrowserPhase::Opening;
                            s.status.error = None;
                            s.status.url = None;
                            s.status.service = Some(service);
                            s.status.engine = "webkit";
                            s.generation
                        };
                        #[cfg(target_os = "macos")]
                        if macos::open(app, state, generation, url, false).is_err() {
                            state.fail(generation);
                        }
                        // A completion that never arrives cannot leave an indefinitely 'opening' view.
                        let timeout_state = state.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                            timeout_state.expire(generation);
                        });
                    }
                }
                BrowserEngine::Chromium => {
                    open_chromium(state, service, url)?;
                }
                BrowserEngine::Scrapling => {
                    // Handled asynchronously in `perform`; the main thread
                    // never blocks on engine startup or page fetches.
                    return Err(AppError::new("browser_unavailable", GATE_ERROR));
                }
            }
        }
        Action::Navigate(raw) => {
            let url = state.validate_url(&raw)?;
            let window = secured_window(app, state)?;
            window.navigate(url).map_err(|_| {
                AppError::new(
                    "browser_navigation_failed",
                    "The browser could not navigate.",
                )
            })?;
            state.lock()?.status.service = Some(BrowserService::Custom);
        }
        Action::History(_) => {
            return Err(AppError::new(
                "browser_navigation_failed",
                "The browser history action must use the native result channel.",
            ));
        }
    }
    if changed {
        state.emit();
    }
    state.status()
}

/// Fail closed before profile access or starting any child, on every platform.
fn open_chromium(_state: &BrowserState, _service: BrowserService, _target: Url) -> AppResult<()> {
    chromium::require_available()
}

fn secured_window(app: &AppHandle, state: &BrowserState) -> AppResult<WebviewWindow> {
    if state.lock()?.status.engine == "chromium" {
        chromium::require_available()?;
    }
    if !state.lock()?.secured {
        return Err(AppError::new(
            "browser_closed",
            "Open the owned browser first.",
        ));
    }
    app.get_webview_window(LABEL)
        .ok_or_else(|| AppError::new("browser_closed", "Open the owned browser first."))
}

// Every command independently enforces the unchanged first-party label+origin guard.
#[tauri::command]
pub async fn browser_status(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, BrowserState>,
) -> AppResult<OwnedBrowserStatus> {
    crate::first_party(&window)?;
    perform(app, state.inner().clone(), Action::Status).await
}
#[tauri::command]
pub async fn browser_open(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, BrowserState>,
    service: BrowserService,
    url: Option<String>,
    engine: Option<BrowserEngine>,
) -> AppResult<OwnedBrowserStatus> {
    crate::first_party(&window)?;
    let advertised = state
        .lock()
        .map(|s| s.status.available_engines.clone())
        .unwrap_or_default();
    let engine = resolve_engine(engine, &advertised)?;
    perform(
        app,
        state.inner().clone(),
        Action::Open(service, url, engine),
    )
    .await
}
#[tauri::command]
pub async fn browser_navigate(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, BrowserState>,
    url: String,
) -> AppResult<OwnedBrowserStatus> {
    crate::first_party(&window)?;
    perform(app, state.inner().clone(), Action::Navigate(url)).await
}
#[tauri::command]
pub async fn browser_close(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, BrowserState>,
) -> AppResult<OwnedBrowserStatus> {
    crate::first_party(&window)?;
    perform(app, state.inner().clone(), Action::Close).await
}
#[tauri::command]
pub async fn browser_back(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, BrowserState>,
) -> AppResult<OwnedBrowserStatus> {
    crate::first_party(&window)?;
    perform(app, state.inner().clone(), Action::History(History::Back)).await
}
#[tauri::command]
pub async fn browser_forward(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, BrowserState>,
) -> AppResult<OwnedBrowserStatus> {
    crate::first_party(&window)?;
    perform(
        app,
        state.inner().clone(),
        Action::History(History::Forward),
    )
    .await
}
#[tauri::command]
pub async fn browser_reload(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, BrowserState>,
) -> AppResult<OwnedBrowserStatus> {
    crate::first_party(&window)?;
    perform(app, state.inner().clone(), Action::History(History::Reload)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_snapshots_have_monotonic_revisions() {
        let state = BrowserState::new(None, false, None);
        let first = state.status().unwrap();
        state.closed(0, false);
        let second = state.status().unwrap();
        assert!(second.revision > first.revision);
        assert_eq!(second.phase, BrowserPhase::Closed);
    }
    #[test]
    fn rejects_unsafe_urls_without_contacting_them() {
        for raw in [
            "https://HIGGSFIELD.ai",
            "https://a.higgsfield.com/",
            "https://higgsfield.io./",
            "https://cdn-higgsfield.example/",
            "https://%68iggsfield.ai/",
            "javascript:alert(1)",
            "file:///tmp/a",
            "data:text/html,hi",
            "about:blank",
            "https://user:secret@example.com",
            "https://@example.com",
            "https://example.com\n",
            "https://example.com\\@evil.test",
            "http://example.com",
            "http://127.0.0.1:9000",
            "https://tauri.localhost/",
        ] {
            assert!(validate_url(raw, false).is_err(), "{raw}");
        }
        assert!(validate_url(
            &format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES)),
            false
        )
        .is_err());
        assert!(validate_url("https://example.com/path?code=private#key", false).is_ok());
    }
    #[test]
    fn snapshot_navigation_is_session_scoped_and_engine_gated() {
        let state = BrowserState::new(None, false, None);
        let session = uuid::Uuid::new_v4();
        let stored = Url::parse("https://example.com/page").unwrap();
        {
            let mut s = state.lock().unwrap();
            s.secured = true;
            s.status.engine = ENGINE_SCRAPLING;
            s.status.phase = BrowserPhase::Open;
        }
        // The store lock is never held across navigation-gate decisions.
        let id = {
            let mut store = state.snapshots.lock().unwrap();
            store.session = Some(session);
            store.insert(stored.clone(), "<html>x</html>".into())
        };
        let url_for = |entry: &str| {
            Url::parse(&format!(
                "{}://{}/{}",
                engine::SNAPSHOT_SCHEME,
                session,
                entry
            ))
            .unwrap()
        };
        assert!(state.permits_snapshot(0, &url_for(&id)));
        assert!(state.permits_snapshot(0, &url_for("home")));
        // Unknown snapshot, foreign session, nested paths, and unsecured
        // sessions never load anything.
        assert!(!state.permits_snapshot(0, &url_for("missing")));
        assert!(!state.permits_snapshot(
            0,
            &Url::parse(&format!(
                "{}://{}/{}",
                engine::SNAPSHOT_SCHEME,
                uuid::Uuid::new_v4(),
                id
            ))
            .unwrap()
        ));
        assert!(!state.permits_snapshot(0, &url_for("a/b")));
        // A stale generation is never permitted, even mid-session.
        assert!(!state.permits(999, &url_for("home")));
        // A secured engine session denies direct loads of external URLs (they
        // are re-fetched through the engine instead) and allows snapshots.
        {
            let mut s = state.lock().unwrap();
            s.secured = true;
            s.status.engine = ENGINE_SCRAPLING;
            s.status.phase = BrowserPhase::Open;
        }
        assert!(!state.permits(0, &Url::parse("https://example.com/").unwrap()));
        assert!(state.permits(
            0,
            &Url::parse(&format!("{}://{}/home", engine::SNAPSHOT_SCHEME, session)).unwrap()
        ));
        // WebKit sessions keep the classic deny-by-default external policy.
        {
            let mut s = state.lock().unwrap();
            s.status.engine = "webkit";
        }
        assert!(state.permits(0, &Url::parse("https://example.com/").unwrap()));
        assert!(!state.permits(0, &Url::parse("http://127.0.0.1:9200/").unwrap()));
    }

    #[test]
    fn scrapling_default_is_first_advertised_engine_only() {
        let engines = vec![
            AvailableEngine {
                id: ENGINE_SCRAPLING,
                label: "Scrapling (Chromium snapshots)",
            },
            AvailableEngine {
                id: "webkit",
                label: "WebKit",
            },
        ];
        assert_eq!(
            resolve_engine(None, &engines).unwrap(),
            BrowserEngine::Scrapling
        );
        assert_eq!(
            resolve_engine(None, &[engines[1].clone()]).unwrap(),
            BrowserEngine::Webkit
        );
        assert!(resolve_engine(Some(BrowserEngine::Chromium), &engines).is_err());
        assert!(resolve_engine(Some(BrowserEngine::Scrapling), &[engines[1].clone()]).is_err());
        assert_eq!(
            resolve_engine(Some(BrowserEngine::Webkit), &engines).unwrap(),
            BrowserEngine::Webkit
        );
    }

    #[test]
    fn loopback_requires_explicit_qa_mode() {
        for url in [
            "http://localhost:9200/",
            "http://127.0.0.1:9200/",
            "http://[::1]:9200/",
        ] {
            assert!(validate_url(url, true).is_ok());
            assert!(validate_url(url, false).is_err());
        }
        assert!(validate_url("http://localhost.evil.test/", true).is_err());
        assert!(validate_url("http://100.90.255.43/", true).is_err());
    }
    #[test]
    fn release_callback_exception_is_narrow_and_fail_closed() {
        let callback = "http://127.0.0.1:49152/oauth/callback?state=synthetic&code=fixture";
        let pending = |url: &Url| url.as_str() == callback;
        assert!(validate_url_with_callback(callback, false, pending).is_ok());
        assert!(validate_url_with_callback(callback, false, |_| false).is_err());
        assert!(BrowserState::new(None, false, None)
            .validate_url(callback)
            .is_err());
        for raw in [
            "http://127.0.0.1:49153/oauth/callback?state=synthetic&code=fixture",
            "http://127.0.0.1:49152/other?state=synthetic&code=fixture",
            "http://127.0.0.1:49152/oauth/callback?state=wrong&code=fixture",
            "https://127.0.0.1:49152/oauth/callback?state=synthetic",
            "http://localhost:49152/oauth/callback?state=synthetic",
            "http://[::1]:49152/oauth/callback?state=synthetic",
            "http://user@127.0.0.1:49152/oauth/callback?state=synthetic",
        ] {
            assert!(
                validate_url_with_callback(raw, false, pending).is_err(),
                "{raw}"
            );
        }
        // Even a buggy matcher cannot waive URL hygiene, scheme or host limits.
        for raw in [
            "http://localhost/",
            "http://example.test/",
            "https://127.0.0.1/",
            "http://@127.0.0.1/",
            "file:///tmp/fixture",
        ] {
            assert!(
                validate_url_with_callback(raw, false, |_| true).is_err(),
                "{raw}"
            );
        }
    }

    #[test]
    fn chromium_cannot_start_or_mutate_an_existing_session() {
        let state = BrowserState::new(None, false, None);
        let before = state.status().unwrap();
        let error = open_chromium(
            &state,
            BrowserService::Home,
            Url::parse("about:blank").unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.code, "browser_unavailable");
        assert_eq!(error.message, chromium::UNAVAILABLE_REASON);
        assert_eq!(
            resolve_engine(
                Some(BrowserEngine::Chromium),
                &[AvailableEngine {
                    id: "webkit",
                    label: "WebKit",
                }]
            )
            .unwrap_err()
            .message,
            chromium::UNAVAILABLE_REASON
        );
        let after = state.status().unwrap();
        assert_eq!(after.phase, before.phase);
        assert_eq!(after.engine, before.engine);
        assert_eq!(after.persistent, before.persistent);
        assert!(!after
            .available_engines
            .iter()
            .any(|engine| engine.id == "chromium"));
        assert_eq!(
            after.chromium_unavailable_reason,
            chromium::UNAVAILABLE_REASON
        );
    }

    #[test]
    fn services_have_fixed_entrypoints() {
        assert_eq!(
            service_url(BrowserService::Home, None, false)
                .unwrap()
                .as_str(),
            "about:blank"
        );
        assert_eq!(
            service_url(BrowserService::Gmail, None, false)
                .unwrap()
                .host_str(),
            Some("mail.google.com")
        );
        assert_eq!(
            service_url(BrowserService::GoogleCalendar, None, false)
                .unwrap()
                .host_str(),
            Some("calendar.google.com")
        );
        assert!(service_url(BrowserService::Home, Some("https://example.com"), false).is_err());
        assert!(service_url(BrowserService::Custom, None, false).is_err());
    }
    #[test]
    fn status_redacts_even_path_secrets() {
        assert_eq!(
            public_url(
                &Url::parse("https://example.com/token/secret?code=secret#access_token=secret")
                    .unwrap()
            )
            .as_deref(),
            Some("https://example.com/")
        );
        let state = BrowserState::new(None, false, None);
        let json = serde_json::to_value(state.status().unwrap()).unwrap();
        assert_eq!(json["automationReady"], false);
        assert_eq!(json["persistent"], false);
        assert!(json["profileId"].is_null());
        assert!(json.get("automation_ready").is_none());
    }
    #[test]
    fn blocker_is_case_insensitive_and_covers_every_resource_type() {
        let rules: serde_json::Value = serde_json::from_str(&content_rules()).unwrap();
        assert_eq!(rules[0]["trigger"]["url-filter"], ".*higgsfield.*");
        assert_eq!(rules[0]["trigger"]["url-filter-is-case-sensitive"], false);
        assert!(rules[0]["trigger"].get("resource-type").is_none());
        assert_eq!(rules[0]["action"]["type"], "block");
    }
    #[test]
    fn timeout_cannot_fail_a_completed_or_newer_generation() {
        let state = BrowserState::new(None, false, None);
        {
            let mut s = state.lock().unwrap();
            s.generation = 7;
            s.status.phase = BrowserPhase::Open;
            s.secured = true;
        }
        state.expire(7);
        assert_eq!(state.status().unwrap().phase, BrowserPhase::Open);
        state.lock().unwrap().status.phase = BrowserPhase::Opening;
        state.expire(6);
        assert!(state.current(7));
        state.expire(7);
        assert_eq!(state.status().unwrap().phase, BrowserPhase::Error);
        assert!(!state.lock().unwrap().secured);
        state.closed(7, true);
        assert_eq!(state.status().unwrap().phase, BrowserPhase::Error);
        state.shutdown();
        assert_eq!(state.status().unwrap().phase, BrowserPhase::Closed);
    }
    /// Status snapshots include the host's engine list under the camelCase
    /// wire name `availableEngines`. WebKit is always the first entry on a
    /// macOS 14+ dev host so the frontend defaults to the truthful label.
    #[test]
    fn status_exposes_available_engines_under_camel_case() {
        let state = BrowserState::new(None, false, None);
        let json = serde_json::to_value(state.status().unwrap()).unwrap();
        let engines = json["availableEngines"].as_array().expect("array present");
        assert!(json.get("available_engines").is_none());
        // The dev/CI host is macOS 14+, so the first engine is WebKit.
        #[cfg(target_os = "macos")]
        {
            let webkit = engines.iter().find(|e| e["id"] == "webkit").unwrap();
            assert_eq!(webkit["label"], "WebKit");
        }
        // Never advertise an engine that is not "webkit" or "chromium".
        for engine in engines {
            let id = engine["id"].as_str().unwrap();
            assert!(id == "webkit" || id == "chromium", "unexpected id: {id}");
        }
    }

    /// `resolve_engine(None)` defaults to WebKit when WebKit is available.
    /// The default is the first advertised engine and never silently
    /// substitutes: a WebKit-only host defaults to WebKit.
    #[test]
    fn resolve_engine_defaults_to_webkit_when_alone() {
        let webkit_only = [AvailableEngine {
            id: "webkit",
            label: "WebKit",
        }];
        assert_eq!(
            resolve_engine(None, &webkit_only).unwrap(),
            BrowserEngine::Webkit
        );
        assert_eq!(
            resolve_engine(Some(BrowserEngine::Webkit), &webkit_only).unwrap(),
            BrowserEngine::Webkit
        );
    }

    /// Requesting an engine the host does not have fails closed with the
    /// canonical `browser_unavailable` code — never falls back silently.
    #[test]
    fn resolve_engine_fails_closed_when_unsupported() {
        // Fabricate the "no chromium on this host" case: on hosts where no
        // Chromium binary is installed under the standard paths, requesting
        // it must return an error. This test is only meaningful when the
        // detection layer reports no chromium engine.
        let engines = browser_engines(None);
        if !engines.iter().any(|e| e.id == "chromium") {
            let err = resolve_engine(Some(BrowserEngine::Chromium), &engines).unwrap_err();
            assert_eq!(err.code, "browser_unavailable");
        }
    }

    /// `browser_engines()` never lies: every reported engine has a truthful
    /// human label and a snake_case id the frontend can key off.
    #[test]
    fn browser_engines_labels_are_truthful() {
        for engine in browser_engines(None) {
            match engine.id {
                "webkit" => assert_eq!(engine.label, "WebKit"),
                "chromium" => assert!(
                    engine.label == "System Chrome"
                        || engine.label == "Chrome Canary"
                        || engine.label == "Chromium",
                    "unexpected chromium label: {}",
                    engine.label
                ),
                other => panic!("unexpected engine id: {other}"),
            }
        }
    }

    #[test]
    fn closing_cancels_late_initialization_and_preserves_profile_identity() {
        let state = BrowserState::new(None, false, None);
        {
            let mut s = state.lock().unwrap();
            s.generation = 4;
            s.status.phase = BrowserPhase::Opening;
            s.status.profile_id = Some("retained".into());
            s.status.persistent = true;
        }
        assert!(state.current(4));
        assert!(!state.permits(4, &Url::parse("https://example.com").unwrap()));
        state.closed(4, false);
        state.fail(4);
        assert!(!state.current(4));
        assert_eq!(state.status().unwrap().phase, BrowserPhase::Closed);
        assert_eq!(
            state.status().unwrap().profile_id.as_deref(),
            Some("retained")
        );
        assert!(state.status().unwrap().persistent);
    }
}
