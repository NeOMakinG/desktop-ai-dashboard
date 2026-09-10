//! Chromium remains unavailable. Post-launch CDP filters cannot protect startup,
//! restored tabs, background requests or new targets before attachment. No child
//! process or pipe transport exists in this runtime. The offline detection,
//! profile and protocol fixtures below are retained research, not security proof.
//! Re-enablement requires independently reviewed pre-launch egress containment.
#![cfg_attr(not(test), allow(dead_code))]

use crate::types::{AppError, AppResult};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
pub(super) const UNAVAILABLE_REASON: &str = "Chromium is unavailable: startup, restored tabs, background traffic and new targets cannot yet be blocked before network access. No Chromium process was started. Use isolated WebKit on macOS 14 or later.";

pub(super) fn require_available() -> AppResult<()> {
    Err(AppError::new("browser_unavailable", UNAVAILABLE_REASON))
}

/// Higgsfield denylist installed via CDP `Network.setBlockedURLs`. Kept as a
/// single wildcard for parity with the WebKit content-blocker rule
/// (`.*higgsfield.*`); Chromium blocklist patterns are glob-style so `*` is
/// the "match anything" wildcard.
pub(super) const HG_BLOCKED_URLS: &[&str] = &["*higgsfield*"];

/// Static launch flags for the managed Chromium child. Order is not important
/// but each flag exists for a specific reason:
/// * `--remote-debugging-pipe` — CDP over fds 3/4; **never** a network port.
/// * `--disable-default-apps` — a fresh profile is not seeded with Chrome apps.
/// * `--no-first-run` — no first-run tab / welcome dialog.
/// * `--no-default-browser-check` — never asks to become the OS default.
/// * `--disable-features=ChromeWhatsNewUI` — no unsolicited What's New page.
pub(super) const LAUNCH_FLAGS: &[&str] = &[
    "--remote-debugging-pipe",
    "--disable-default-apps",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-features=ChromeWhatsNewUI",
];

/// Distinct build we detected on the host. `label` is what the UI displays and
/// must remain truthful — "System Chrome" for Google Chrome, "Chrome Canary"
/// for Canary, "Chromium" for open-source Chromium. Never label a WebKit
/// engine as Chromium (or vice versa).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChromiumInstall {
    pub label: &'static str,
    pub kind: ChromiumKind,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ChromiumKind {
    Chrome,
    ChromeCanary,
    Chromium,
}

/// Ordered list of candidate locations per platform. The first executable
/// build wins for each `ChromiumKind` — so a user-installed Google Chrome is
/// preferred over a bundled path if both existed.
fn candidates() -> Vec<(&'static str, ChromiumKind, PathBuf)> {
    #[allow(unused_mut)]
    let mut list: Vec<(&'static str, ChromiumKind, PathBuf)> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        list.push((
            "System Chrome",
            ChromiumKind::Chrome,
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        ));
        list.push((
            "Chrome Canary",
            ChromiumKind::ChromeCanary,
            PathBuf::from(
                "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            ),
        ));
        list.push((
            "Chromium",
            ChromiumKind::Chromium,
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        ));
    }
    #[cfg(target_os = "linux")]
    {
        for (label, kind, path) in [
            (
                "System Chrome",
                ChromiumKind::Chrome,
                "/usr/bin/google-chrome-stable",
            ),
            (
                "System Chrome",
                ChromiumKind::Chrome,
                "/usr/bin/google-chrome",
            ),
            ("Chromium", ChromiumKind::Chromium, "/usr/bin/chromium"),
            (
                "Chromium",
                ChromiumKind::Chromium,
                "/usr/bin/chromium-browser",
            ),
        ] {
            list.push((label, kind, PathBuf::from(path)));
        }
    }
    #[cfg(target_os = "windows")]
    {
        list.push((
            "System Chrome",
            ChromiumKind::Chrome,
            PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
        ));
        list.push((
            "System Chrome",
            ChromiumKind::Chrome,
            PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
        ));
    }
    list
}

/// Public detection entry point — uses the real filesystem.
pub(super) fn detect() -> Vec<ChromiumInstall> {
    detect_from(&candidates(), &is_executable)
}

/// Testable detection kernel — the exists predicate is injected so unit tests
/// never touch `/Applications/`. Keeps only the first hit per `ChromiumKind`.
pub(super) fn detect_from(
    candidates: &[(&'static str, ChromiumKind, PathBuf)],
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<ChromiumInstall> {
    use std::collections::BTreeSet;
    let mut seen: BTreeSet<ChromiumKind> = BTreeSet::new();
    let mut engines: Vec<ChromiumInstall> = Vec::new();
    for (label, kind, path) in candidates {
        if seen.contains(kind) {
            continue;
        }
        if !exists(path) {
            continue;
        }
        seen.insert(*kind);
        engines.push(ChromiumInstall {
            label,
            kind: *kind,
            path: path.clone(),
        });
    }
    engines
}

/// True when the path exists as a regular file with any executable bit set on
/// Unix, or exists at all on Windows.
pub(super) fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                metadata.file_type().is_file() && metadata.permissions().mode() & 0o111 != 0
            }
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        fs::symlink_metadata(path)
            .map(|m| m.file_type().is_file())
            .unwrap_or(false)
    }
}

/// Root directory Forma owns for managed-Chromium user-data. Distinct from
/// `owned-browser/` so the WebKit engine's profile never mixes with Chromium
/// state and vice versa.
pub(super) fn user_data_root(app_data: &Path) -> PathBuf {
    app_data.join("managed-chromium")
}

/// Prepare (create if missing) the Forma-owned user-data-dir. Refuses to
/// operate on symlinks, world-readable directories, or paths owned by another
/// UID — the same posture `profile.rs` uses for the WebKit profile-id file.
/// **Never** returns a path that could resolve into the user's normal Chrome
/// profile (`~/Library/Application Support/Google/Chrome`), because we only
/// construct paths under `<app_data>/managed-chromium/`.
#[cfg(unix)]
pub(super) fn prepare_user_data_dir(app_data: &Path) -> AppResult<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if !app_data.is_absolute()
        || !fs::symlink_metadata(app_data)
            .map_err(|_| unavailable())?
            .is_dir()
    {
        return Err(unavailable());
    }
    let directory = user_data_root(app_data);
    match fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(unavailable()),
    }
    let metadata = fs::symlink_metadata(&directory).map_err(|_| unavailable())?;
    if !metadata.is_dir()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(unavailable());
    }
    Ok(directory)
}

#[cfg(not(unix))]
pub(super) fn prepare_user_data_dir(app_data: &Path) -> AppResult<PathBuf> {
    if !app_data.is_absolute() {
        return Err(unavailable());
    }
    let directory = user_data_root(app_data);
    if let Err(e) = fs::create_dir_all(&directory) {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(unavailable());
        }
    }
    Ok(directory)
}

fn unavailable() -> AppError {
    AppError::new("browser_unavailable", super::GATE_ERROR)
}

/// Build the argv (arguments only — not argv[0]) for the Chromium child.
/// Isolated from spawning so tests can inspect the command line without ever
/// launching a browser.
pub(super) fn build_argv(user_data_dir: &Path, target_url: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = Vec::with_capacity(1 + LAUNCH_FLAGS.len() + 1);
    argv.push(format!("--user-data-dir={}", user_data_dir.display()));
    for flag in LAUNCH_FLAGS {
        argv.push((*flag).to_string());
    }
    if let Some(url) = target_url {
        argv.push(url.to_string());
    }
    argv
}

/// CDP wire format for `--remote-debugging-pipe`: each direction is a
/// concatenation of UTF-8 JSON objects, each terminated by a single NUL byte.
/// Returns owned bytes so the caller can hand them straight to a pipe writer.
pub(super) fn frame(message: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(message.len() + 1);
    bytes.extend_from_slice(message.as_bytes());
    bytes.push(0);
    bytes
}

/// Split an input buffer at NUL boundaries. Ignores an empty trailing frame
/// produced by a still-open stream. Never allocates the message bodies.
pub(super) fn split_frames(buffer: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (index, byte) in buffer.iter().enumerate() {
        if *byte == 0 {
            if start < index {
                out.push(&buffer[start..index]);
            }
            start = index + 1;
        }
    }
    out
}

/// Offline protocol fixture: Network commands require an attached target's
/// flat session. The browser/root session does not support this domain.
pub(super) fn blocked_urls_message(id: u64, session_id: &str) -> String {
    #[derive(Serialize)]
    struct Msg<'a> {
        id: u64,
        method: &'a str,
        params: Params<'a>,
        #[serde(rename = "sessionId")]
        session_id: &'a str,
    }
    #[derive(Serialize)]
    struct Params<'a> {
        urls: &'a [&'a str],
    }
    serde_json::to_string(&Msg {
        id,
        method: "Network.setBlockedURLs",
        params: Params {
            urls: HG_BLOCKED_URLS,
        },
        session_id,
    })
    .expect("static CDP message serializes")
}

/// Offline protocol transport; tests buffer messages in memory. There is no
/// runtime pipe implementation. A future implementation needs bounded,
/// cancellable reads/writes and supervised child reaping before enablement.
pub(super) trait CdpTransport {
    fn write_message(&mut self, message: &str) -> std::io::Result<()>;
    fn read_message(&mut self) -> std::io::Result<Option<String>>;
}

/// Send `Network.setBlockedURLs(HG_BLOCKED_URLS)` and read one response with a
/// matching `id`. Returns an error if the response is a CDP `error` object or
/// if the stream ends before a matching reply arrives.
pub(super) fn install_higgsfield_block(
    transport: &mut dyn CdpTransport,
    id_gen: &AtomicU64,
    session_id: &str,
) -> std::io::Result<()> {
    if session_id.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "CDP target session required",
        ));
    }
    let id = id_gen.fetch_add(1, Ordering::SeqCst);
    let message = blocked_urls_message(id, session_id);
    transport.write_message(&message)?;
    loop {
        let reply = transport.read_message()?;
        let Some(text) = reply else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "CDP stream ended before blocked-URL ack",
            ));
        };
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "malformed CDP"))?;
        if value.get("id").and_then(|v| v.as_u64()) != Some(id) {
            // Ignore events (`method` w/o `id`) and unrelated acks.
            continue;
        }
        if value.get("sessionId").and_then(|v| v.as_str()) != Some(session_id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "CDP target session mismatch",
            ));
        }
        if value.get("error").is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "CDP rejected Network.setBlockedURLs",
            ));
        }
        return Ok(());
    }
}

// ---------------------------------------------------------------------------
// Tests — pure Rust; NO runtime spawn, NO Higgsfield calls, NO network I/O.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn chromium_is_always_unavailable_before_any_side_effect() {
        let error = require_available().unwrap_err();
        assert_eq!(error.code, "browser_unavailable");
        assert_eq!(error.message, UNAVAILABLE_REASON);
    }

    #[test]
    fn target_session_is_required_before_sending_a_network_command() {
        let mut transport = StubTransport::new(vec![]);
        let error = install_higgsfield_block(&mut transport, &AtomicU64::new(1), "").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(transport.writes.is_empty());
    }

    #[test]
    fn root_or_wrong_target_acknowledgment_cannot_secure_a_target() {
        for reply in [
            r#"{"id":1,"result":{}}"#,
            r#"{"id":1,"sessionId":"OTHER","result":{}}"#,
        ] {
            let mut transport = StubTransport::new(vec![reply]);
            let error =
                install_higgsfield_block(&mut transport, &AtomicU64::new(1), "SID").unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        }
    }

    /// Detection is deterministic, deduplicates by kind (first-in wins), and
    /// only returns entries the exists predicate accepts. Never touches the
    /// real filesystem.
    #[test]
    fn detects_first_hit_per_kind_and_ignores_missing_binaries() {
        let candidates = vec![
            (
                "System Chrome",
                ChromiumKind::Chrome,
                PathBuf::from("/present/google-chrome"),
            ),
            (
                "System Chrome",
                ChromiumKind::Chrome,
                PathBuf::from("/also-present/google-chrome-fallback"),
            ),
            (
                "Chrome Canary",
                ChromiumKind::ChromeCanary,
                PathBuf::from("/absent/canary"),
            ),
            (
                "Chromium",
                ChromiumKind::Chromium,
                PathBuf::from("/present/chromium"),
            ),
        ];
        let exists = |p: &Path| p.starts_with("/present") || p.starts_with("/also-present");
        let engines = detect_from(&candidates, &exists);
        // Chrome deduplicated to the first hit; Chromium included; Canary skipped.
        assert_eq!(engines.len(), 2);
        assert_eq!(engines[0].kind, ChromiumKind::Chrome);
        assert_eq!(engines[0].label, "System Chrome");
        assert_eq!(engines[0].path, PathBuf::from("/present/google-chrome"));
        assert_eq!(engines[1].kind, ChromiumKind::Chromium);
        assert_eq!(engines[1].label, "Chromium");
    }

    /// `detect_from` on an empty exists-predicate returns an empty vec, so
    /// callers correctly fail closed on platforms with no supported binary.
    #[test]
    fn empty_when_no_binary_exists() {
        let candidates = vec![(
            "System Chrome",
            ChromiumKind::Chrome,
            PathBuf::from("/nope"),
        )];
        assert!(detect_from(&candidates, &|_p: &Path| false).is_empty());
    }

    /// The real `is_executable` accepts a chmod +x file, rejects a plain file,
    /// and rejects a missing path. Uses tempfile so nothing outside the test
    /// tree is touched.
    #[cfg(unix)]
    #[test]
    fn is_executable_respects_mode_bits() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain");
        fs::write(&plain, b"#!/bin/sh\n").unwrap();
        assert!(!is_executable(&plain));
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable(&plain));
        assert!(!is_executable(&dir.path().join("missing")));
    }

    /// argv always contains the Forma-owned user-data-dir first (so a stray
    /// later flag can't accidentally override it), every LAUNCH_FLAG, and
    /// never `--remote-debugging-port`.
    #[test]
    fn argv_pins_user_data_dir_and_uses_pipe_never_port() {
        let dir = PathBuf::from("/var/app-data/managed-chromium");
        let argv = build_argv(&dir, Some("https://mail.google.com/"));
        assert_eq!(
            argv[0], "--user-data-dir=/var/app-data/managed-chromium",
            "user-data-dir must be first so nothing overrides it"
        );
        assert!(argv.iter().any(|a| a == "--remote-debugging-pipe"));
        assert!(argv.iter().any(|a| a == "--disable-default-apps"));
        assert!(argv.iter().any(|a| a == "--no-first-run"));
        assert!(argv.iter().any(|a| a == "--no-default-browser-check"));
        assert!(argv
            .iter()
            .any(|a| a == "--disable-features=ChromeWhatsNewUI"));
        assert!(
            !argv
                .iter()
                .any(|a| a.starts_with("--remote-debugging-port")),
            "port-based CDP is a network attack surface and must never be enabled"
        );
        assert_eq!(argv.last().unwrap(), "https://mail.google.com/");
    }

    /// argv omits the trailing URL when none is requested.
    #[test]
    fn argv_omits_url_when_none() {
        let argv = build_argv(&PathBuf::from("/x"), None);
        assert!(!argv.iter().any(|a| a.starts_with("http")));
    }

    /// Blocked-URL config is exactly the Higgsfield wildcard, in the shape
    /// CDP expects. Never returns any other host.
    #[test]
    fn blocked_urls_message_is_higgsfield_only() {
        let msg = blocked_urls_message(7, "SID");
        let value: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(value["id"], 7);
        assert_eq!(value["method"], "Network.setBlockedURLs");
        assert_eq!(value["sessionId"], "SID");
        let urls = value["params"]["urls"].as_array().unwrap();
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "*higgsfield*");
        // No other host is silently added.
        for banned in ["mail.google.com", "example.com", "openai.com"] {
            assert!(!msg.contains(banned));
        }
    }

    /// Frames are NUL-terminated and split back correctly, including on
    /// concatenated buffers.
    #[test]
    fn framing_is_nul_delimited() {
        let a = frame("{\"id\":1}");
        assert_eq!(*a.last().unwrap(), 0);
        let mut combined = frame("{\"id\":1}");
        combined.extend_from_slice(&frame("{\"id\":2}"));
        let parts = split_frames(&combined);
        assert_eq!(parts.len(), 2);
        assert_eq!(std::str::from_utf8(parts[0]).unwrap(), "{\"id\":1}");
        assert_eq!(std::str::from_utf8(parts[1]).unwrap(), "{\"id\":2}");
    }

    /// Stub transport that flushes queued replies whenever the code under
    /// test writes a message; captures every write for later assertion.
    struct StubTransport {
        writes: Vec<String>,
        replies: VecDeque<String>,
    }
    impl StubTransport {
        fn new(replies: Vec<&str>) -> Self {
            Self {
                writes: Vec::new(),
                replies: replies.into_iter().map(String::from).collect(),
            }
        }
    }
    impl CdpTransport for StubTransport {
        fn write_message(&mut self, message: &str) -> std::io::Result<()> {
            self.writes.push(message.to_string());
            Ok(())
        }
        fn read_message(&mut self) -> std::io::Result<Option<String>> {
            Ok(self.replies.pop_front())
        }
    }

    /// A successful handshake writes the Higgsfield block message, matches
    /// the reply id, and returns Ok. The exact id is monotonic and starts at
    /// 1 (fetch_add returns the pre-increment value).
    #[test]
    fn handshake_installs_higgsfield_block_and_matches_reply_id() {
        let mut transport = StubTransport::new(vec![
            "{\"method\":\"Target.attachedToTarget\",\"params\":{}}", // an event; ignored
            "{\"id\":1,\"sessionId\":\"SID\",\"result\":{}}",
        ]);
        let ids = AtomicU64::new(1);
        install_higgsfield_block(&mut transport, &ids, "SID").unwrap();
        assert_eq!(transport.writes.len(), 1);
        let sent: serde_json::Value = serde_json::from_str(&transport.writes[0]).unwrap();
        assert_eq!(sent["id"], 1);
        assert_eq!(sent["method"], "Network.setBlockedURLs");
        assert_eq!(sent["params"]["urls"][0], "*higgsfield*");
        assert_eq!(sent["sessionId"], "SID");
        // Subsequent handshakes advance the id.
        let mut transport =
            StubTransport::new(vec!["{\"id\":2,\"sessionId\":\"SID\",\"result\":{}}"]);
        install_higgsfield_block(&mut transport, &ids, "SID").unwrap();
        let sent: serde_json::Value = serde_json::from_str(&transport.writes[0]).unwrap();
        assert_eq!(sent["id"], 2);
    }

    /// A CDP `error` reply fails the handshake — we must not silently continue
    /// launching Chromium without a Higgsfield block in place.
    #[test]
    fn handshake_fails_when_cdp_rejects_block() {
        let mut transport = StubTransport::new(vec![
            "{\"id\":9,\"sessionId\":\"SID\",\"error\":{\"code\":-32601,\"message\":\"unknown\"}}",
        ]);
        let ids = AtomicU64::new(9);
        assert!(install_higgsfield_block(&mut transport, &ids, "SID").is_err());
    }

    /// The transport ending mid-handshake fails closed (never returns Ok
    /// without a matched ack).
    #[test]
    fn handshake_fails_when_stream_ends() {
        let mut transport = StubTransport::new(vec![]);
        let ids = AtomicU64::new(1);
        let err = install_higgsfield_block(&mut transport, &ids, "SID").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    /// `user_data_root` composes safely and only under `<app_data>/`.
    #[test]
    fn user_data_root_lives_under_app_data() {
        let root = user_data_root(Path::new("/tmp/appdata"));
        assert!(root.starts_with("/tmp/appdata"));
        assert!(root.ends_with("managed-chromium"));
    }

    /// `prepare_user_data_dir` creates a 0700 directory when missing, refuses
    /// symlinked replacements, and never widens permissions on an existing
    /// directory it accepts.
    #[cfg(unix)]
    #[test]
    fn prepare_user_data_dir_creates_and_refuses_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = prepare_user_data_dir(root.path()).unwrap();
        assert!(dir.is_dir());
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // Idempotent when already present.
        assert_eq!(prepare_user_data_dir(root.path()).unwrap(), dir);
        // Relative paths refused.
        assert!(prepare_user_data_dir(Path::new("relative")).is_err());

        // Symlink at the target — never dereferenced into another dir.
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("managed-chromium")).unwrap();
        assert!(prepare_user_data_dir(root.path()).is_err());
    }
}
