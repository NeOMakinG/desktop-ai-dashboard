//! REAL Chrome-for-Testing engine (founder directive 2026-09-13).
//!
//! One real, headful Chromium that the user browses and signs into directly,
//! and that the assistant may drive over local CDP only when the user enables
//! it. Deliberate design decisions, each the opposite of the old stub:
//! * FULL Chrome for Testing, pinned by version + sha256, downloaded on first
//!   use into the app data dir and extracted with `/usr/bin/ditto -x -k`
//!   (preserves the .app framework symlinks that break naive bundling).
//! * NO headless flags and NO proxy: the whole point is the user's own
//!   browser fingerprint on the user's own residential IP. Routing this
//!   browser through a relay or shared egress would defeat the ban-resistant
//!   design, so egress policy is intentionally the OS default.
//! * CDP on an ephemeral loopback port (`--remote-debugging-port=0`; Chrome
//!   binds 127.0.0.1 and writes the port to `DevToolsActivePort` inside the
//!   profile). The endpoint is handed to the runtime ONLY while the browser
//!   runs AND the user has switched on "assistant may drive the browser"
//!   (default OFF, stored with the other preferences).
//! * The profile persists under the app data dir, so the user's sign-ins
//!   survive restarts. Forma never copies or reads other browsers' profiles.
//! * The child is not killed when a chat ends (`kill_on_drop(false)` — the
//!   browser may outlive a conversation) but is reaped on app quit through
//!   the same owned-process discipline as every other Forma child.
use crate::types::{AppError, AppResult};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

/// Pinned Chrome for Testing build. Verified 2026-09-13: the CDN serves this
/// exact zip (187,406,357 bytes) and the extracted binary reports
/// "Google Chrome for Testing 151.0.7922.34".
pub(crate) const CFT_VERSION: &str = "151.0.7922.34";
pub(crate) const CFT_URL: &str =
    "https://cdn.playwright.dev/builds/cft/151.0.7922.34/mac-arm64/chrome-mac-arm64.zip";
pub(crate) const CFT_SHA256: &str =
    "01a23ef9501b2745e0c2944c2e583207e6f6132d8d91c3a87ff65b5079e438ef";
const CFT_EXECUTABLE_RELATIVE: &str =
    "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing";

pub(crate) const ENGINE_ID: &str = "chromium";
pub(crate) const ENGINE_LABEL: &str = "Chrome (full browser)";
pub(super) const PLATFORM_UNAVAILABLE: &str =
    "The full Chrome browser is packaged for Apple silicon macOS only.";

pub(super) fn supported() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

// ---------------------------------------------------------------------------
// Owned paths — everything lives under `<app data>/real-browser/`.
// ---------------------------------------------------------------------------

pub(super) fn real_root(app_data: &Path) -> PathBuf {
    app_data.join("real-browser")
}

pub(super) fn install_root(app_data: &Path) -> PathBuf {
    real_root(app_data).join("chromium-151")
}

pub(super) fn executable_path(app_data: &Path) -> PathBuf {
    install_root(app_data).join(CFT_EXECUTABLE_RELATIVE)
}

/// Persistent profile: the user's real sign-ins live here and survive
/// restarts. Never a temp dir, never another browser's profile.
pub(super) fn profile_dir(app_data: &Path) -> PathBuf {
    real_root(app_data).join("profiles").join("default")
}

fn zip_path(app_data: &Path) -> PathBuf {
    real_root(app_data).join("downloads").join("chrome-mac-arm64.zip")
}

pub(super) fn installed(app_data: &Path) -> bool {
    let path = executable_path(app_data);
    fs::metadata(&path).map(|m| m.is_file()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Binary preference (stealth addendum 2026-09-13): prefer the user's OWN
// installed Google Chrome — Chrome for Testing exposes the literal brand
// "Chrome for Testing" in `navigator.userAgentData.brands`, a direct
// automation tell. The real Chrome binary carries real brands. We still use
// OUR separate `--user-data-dir` profile, never the user's default profile
// (which since Chrome 136 also refuses remote debugging — ours does not).
// The bundled Chrome for Testing remains the fallback when no system Chrome
// is installed. The chosen binary is reported honestly in the status card.
// ---------------------------------------------------------------------------

pub(crate) const SYSTEM_CHROME_BINARY: &str =
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
pub(crate) const BINARY_SYSTEM: &str = "system_chrome";
pub(crate) const BINARY_TESTING: &str = "chrome_for_testing";

pub(super) fn system_chrome_installed() -> bool {
    supported()
        && fs::metadata(SYSTEM_CHROME_BINARY)
            .map(|m| m.is_file())
            .unwrap_or(false)
}

/// Which binary would launch right now, without downloading anything.
pub(crate) fn binary_kind(app_data: Option<&Path>) -> Option<&'static str> {
    if system_chrome_installed() {
        Some(BINARY_SYSTEM)
    } else if app_data.map(installed).unwrap_or(false) {
        Some(BINARY_TESTING)
    } else {
        None
    }
}

/// True when the engine can open without a first-use download.
pub(super) fn openable(app_data: Option<&Path>) -> bool {
    binary_kind(app_data).is_some()
}

/// Resolve the executable to launch, preferring the system Chrome.
pub(super) fn resolve_binary(app_data: &Path) -> Option<PathBuf> {
    if system_chrome_installed() {
        Some(PathBuf::from(SYSTEM_CHROME_BINARY))
    } else if installed(app_data) {
        Some(executable_path(app_data))
    } else {
        None
    }
}

/// Hide or reveal the embedded Chrome app (Cmd+H semantics, applied to the
/// child by pid). This is how "entirely inside Forma" works on macOS: the OS
/// clamps fully-offscreen windows back onto the screen, but a hidden app has
/// no visible UI at all while `EMBEDDED_OCCLUSION_FLAG` keeps the screencast
/// streaming. Revealing (pop out) also activates the app so the real window
/// comes to the front. Public NSRunningApplication API — no accessibility
/// permission involved.
#[cfg(target_os = "macos")]
pub(super) fn set_app_hidden(pid: u32, hidden: bool) -> bool {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        return false;
    };
    if hidden {
        app.hide()
    } else {
        let shown = app.unhide();
        app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
        shown
    }
}

// ---------------------------------------------------------------------------
// Launch arguments — isolated from spawning so tests inspect the command line.
// ---------------------------------------------------------------------------

/// Embedded launch (founder directive 2026-09-13): the SAME headful Chrome,
/// but its window starts far offscreen so the user only ever sees Forma's
/// in-app screencast surface. Deliberately NOT `--headless`: the headful
/// fingerprint is the anti-ban design. "Pop out" later repositions this same
/// window onscreen via Browser.setWindowBounds.
pub(super) const EMBEDDED_POSITION_FLAG: &str = "--window-position=20000,20000";
pub(super) const EMBEDDED_SIZE_FLAG: &str = "--window-size=1440,900";
/// macOS clamps fully-offscreen windows back onto the screen, so the embedded
/// window is HIDDEN (NSRunningApplication hide — Cmd+H semantics) right after
/// the CDP surface connects. This flag keeps Chrome compositing (and thus the
/// screencast streaming) while hidden/occluded. It is a process flag only:
/// page JavaScript cannot observe the command line, so it adds no
/// fingerprint surface.
pub(super) const EMBEDDED_OCCLUSION_FLAG: &str = "--disable-backgrounding-occluded-windows";

/// Build argv (arguments only, not argv[0]) for the real browser child.
/// * `--user-data-dir` first so nothing can override the owned profile.
/// * `--remote-debugging-port=0` — ephemeral CDP on 127.0.0.1; Chrome writes
///   the chosen port to `DevToolsActivePort` inside the profile.
/// * NO headless flag and NO `--proxy-server`: user's real window, user's
///   real IP (see module docs — this is the anti-ban design, on purpose).
/// * `embedded` adds only window placement flags (offscreen position + a
///   sane size for the screencast), never rendering-mode flags.
pub(super) fn build_argv(profile: &Path, target_url: Option<&str>, embedded: bool) -> Vec<String> {
    let mut argv = vec![
        format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-port=0".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
    ];
    if embedded {
        argv.push(EMBEDDED_POSITION_FLAG.to_string());
        argv.push(EMBEDDED_SIZE_FLAG.to_string());
        argv.push(EMBEDDED_OCCLUSION_FLAG.to_string());
    }
    if let Some(url) = target_url {
        argv.push(url.to_string());
    }
    argv
}

/// First line of the profile's `DevToolsActivePort` file is the bound port.
pub(super) fn parse_devtools_port(content: &str) -> Option<u16> {
    let port: u16 = content.lines().next()?.trim().parse().ok()?;
    (port != 0).then_some(port)
}

// ---------------------------------------------------------------------------
// Status surfaced to the renderer (BrowserView status card).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RealPhase {
    #[default]
    Idle,
    Downloading,
    Extracting,
    Verifying,
    Launching,
    Running,
    Exited,
    Error,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RealChromiumStatus {
    pub supported: bool,
    /// True when a usable binary is present (system Chrome or the bundled
    /// Chrome for Testing) — no first-use download needed.
    pub installed: bool,
    /// Which binary launches: "system_chrome" (preferred, real brands) or
    /// "chrome_for_testing" (fallback). Honest label for the status card.
    pub binary: Option<&'static str>,
    pub phase: RealPhase,
    /// Download progress while `phase == downloading`.
    pub progress_percent: Option<u8>,
    pub pid: Option<u32>,
    /// True while the loopback CDP endpoint answered `/json/version`.
    pub cdp_ready: bool,
    pub error: Option<&'static str>,
    pub version: &'static str,
}

#[derive(Default)]
struct RealInner {
    phase: RealPhase,
    progress: Option<u8>,
    pid: Option<u32>,
    cdp_port: Option<u16>,
    error: Option<&'static str>,
    busy: bool,
    generation: u64,
}

/// Shared real-browser lifecycle state. Lock discipline mirrors `Session`:
/// short critical sections, never held across awaits.
#[derive(Default)]
pub(crate) struct RealChromium {
    inner: Mutex<RealInner>,
}

impl RealChromium {
    pub(super) fn snapshot(&self, app_data: Option<&Path>) -> RealChromiumStatus {
        let inner = self.inner.lock();
        let (phase, progress, pid, cdp, error) = match &inner {
            Ok(inner) => (
                inner.phase,
                inner.progress,
                inner.pid,
                inner.cdp_port.is_some(),
                inner.error,
            ),
            Err(_) => (RealPhase::Error, None, None, false, Some(super::GATE_ERROR)),
        };
        RealChromiumStatus {
            supported: supported(),
            installed: openable(app_data),
            binary: binary_kind(app_data),
            phase,
            progress_percent: progress,
            pid,
            cdp_ready: cdp,
            error,
            version: CFT_VERSION,
        }
    }

    /// Claim the single open/install slot. Returns the generation for this
    /// attempt, or None when an attempt is already in flight.
    pub(super) fn begin(&self) -> Option<u64> {
        let mut inner = self.inner.lock().ok()?;
        if inner.busy {
            return None;
        }
        inner.busy = true;
        inner.generation = inner.generation.wrapping_add(1);
        inner.error = None;
        Some(inner.generation)
    }

    pub(super) fn progress(&self, generation: u64, phase: RealPhase, percent: Option<u8>) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.generation == generation {
                inner.phase = phase;
                inner.progress = percent;
            }
        }
    }

    pub(super) fn running(&self, generation: u64, pid: u32, cdp_port: u16) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.generation == generation {
                inner.phase = RealPhase::Running;
                inner.progress = None;
                inner.pid = Some(pid);
                inner.cdp_port = Some(cdp_port);
                inner.busy = false;
            }
        }
    }

    pub(super) fn failed(&self, generation: u64, error: &'static str) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.generation == generation {
                inner.phase = RealPhase::Error;
                inner.progress = None;
                inner.pid = None;
                inner.cdp_port = None;
                inner.error = Some(error);
                inner.busy = false;
            }
        }
    }

    /// Child exit observed. Only the generation that launched the child may
    /// transition the state — a stale monitor cannot clobber a relaunch.
    pub(super) fn exited(&self, generation: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.generation == generation {
                inner.phase = RealPhase::Exited;
                inner.pid = None;
                inner.cdp_port = None;
                inner.busy = false;
            }
        }
    }

    /// (pid, cdp_port) while running; verifies the pid is still alive so a
    /// crashed browser is never reported as available.
    pub(super) fn live(&self) -> Option<(u32, u16)> {
        let inner = self.inner.lock().ok()?;
        if inner.phase != RealPhase::Running {
            return None;
        }
        let (pid, port) = (inner.pid?, inner.cdp_port?);
        #[cfg(unix)]
        if unsafe { libc::kill(pid as i32, 0) } != 0 {
            return None;
        }
        Some((pid, port))
    }

    pub(super) fn generation(&self) -> u64 {
        self.inner.lock().map(|inner| inner.generation).unwrap_or(0)
    }

    /// App-quit reaping: the browser never outlives Forma. SIGTERM lets
    /// Chrome flush the profile (sessions persist for next launch).
    pub(super) fn terminate(&self) {
        let pid = self.inner.lock().ok().and_then(|inner| {
            (inner.phase == RealPhase::Running).then_some(inner.pid).flatten()
        });
        #[cfg(unix)]
        if let Some(pid) = pid {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
        #[cfg(not(unix))]
        let _ = pid;
        if let Ok(mut inner) = self.inner.lock() {
            if inner.phase == RealPhase::Running {
                inner.phase = RealPhase::Exited;
                inner.pid = None;
                inner.cdp_port = None;
                inner.busy = false;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime hand-off: {available, cdpEndpoint} for the forma_browser_session
// tool. Fail closed on every missing precondition; never an error the model
// could mistake for a retryable transport fault.
// ---------------------------------------------------------------------------

pub(crate) fn session_payload(cdp_port: Option<u16>, drive_enabled: bool) -> serde_json::Value {
    match (cdp_port, drive_enabled) {
        (Some(port), true) => serde_json::json!({
            "kind": "browserSession",
            "available": true,
            "cdpEndpoint": format!("http://127.0.0.1:{port}"),
        }),
        (Some(_), false) | (None, false) if !drive_enabled => serde_json::json!({
            "kind": "browserSession",
            "available": false,
            "detail": "The user has not enabled assistant browser driving.",
        }),
        _ => serde_json::json!({
            "kind": "browserSession",
            "available": false,
            "detail": "The user's Forma browser is not running.",
        }),
    }
}

// ---------------------------------------------------------------------------
// Install: download → sha256 → ditto extract → binary --version. Blocking
// helpers are wrapped by the async caller in spawn_blocking where needed.
// ---------------------------------------------------------------------------

fn install_error(message: &'static str) -> AppError {
    AppError::new("browser_install_failed", message)
}

const DOWNLOAD_FAILED: &str =
    "The Chrome download could not be completed. Check your connection and try again.";
const VERIFY_FAILED: &str =
    "The downloaded Chrome archive failed its integrity check and was discarded.";
const EXTRACT_FAILED: &str = "The Chrome archive could not be extracted.";
const BINARY_FAILED: &str = "The extracted Chrome binary did not verify.";
pub(super) const LAUNCH_FAILED: &str = "The Chrome browser could not be started.";

/// Download the pinned zip with streaming sha256. Returns the verified zip
/// path. Re-verifies the hash of any previously downloaded zip instead of
/// trusting it. Progress is reported in whole percent.
pub(super) async fn download_zip(
    app_data: &Path,
    report: impl Fn(u8) + Send + Sync,
) -> AppResult<PathBuf> {
    let target = zip_path(app_data);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|_| install_error(DOWNLOAD_FAILED))?;
    }
    if target.is_file() {
        let existing = target.clone();
        let hash = tokio::task::spawn_blocking(move || file_sha256(&existing))
            .await
            .map_err(|_| install_error(VERIFY_FAILED))?;
        if hash.as_deref() == Some(CFT_SHA256) {
            return Ok(target);
        }
        let _ = fs::remove_file(&target);
    }
    let part = target.with_extension("zip.part");
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(1800))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| install_error(DOWNLOAD_FAILED))?
        .get(CFT_URL)
        .send()
        .await
        .map_err(|_| install_error(DOWNLOAD_FAILED))?;
    if !response.status().is_success() {
        return Err(install_error(DOWNLOAD_FAILED));
    }
    let total = response.content_length().unwrap_or(187_406_357);
    let mut file = fs::File::create(&part).map_err(|_| install_error(DOWNLOAD_FAILED))?;
    let mut hash = Sha256::new();
    let mut written: u64 = 0;
    let mut response = response;
    use std::io::Write;
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|_| install_error(DOWNLOAD_FAILED))?;
        let Some(chunk) = chunk else { break };
        hash.update(&chunk);
        file.write_all(&chunk)
            .map_err(|_| install_error(DOWNLOAD_FAILED))?;
        written = written.saturating_add(chunk.len() as u64);
        if written > 400_000_000 {
            // Bounded: a hostile mirror cannot fill the disk.
            let _ = fs::remove_file(&part);
            return Err(install_error(VERIFY_FAILED));
        }
        report(((written.saturating_mul(100)) / total.max(1)).min(100) as u8);
    }
    file.flush().map_err(|_| install_error(DOWNLOAD_FAILED))?;
    drop(file);
    if format!("{:x}", hash.finalize()) != CFT_SHA256 {
        let _ = fs::remove_file(&part);
        return Err(install_error(VERIFY_FAILED));
    }
    fs::rename(&part, &target).map_err(|_| install_error(DOWNLOAD_FAILED))?;
    Ok(target)
}

fn file_sha256(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).ok()?;
    Some(format!("{:x}", hash.finalize()))
}

/// Extract with `/usr/bin/ditto -x -k`: unlike naive unzip crates, ditto
/// preserves the framework symlinks inside the .app (the known bundling
/// failure mode), which is why the browser is installed at first use instead
/// of being shipped inside Forma.app.
pub(super) async fn extract_zip(app_data: &Path, zip: &Path) -> AppResult<()> {
    let destination = install_root(app_data);
    let staging = destination.with_extension("staging");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|_| install_error(EXTRACT_FAILED))?;
    let status = tokio::process::Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(zip)
        .arg(&staging)
        .status()
        .await
        .map_err(|_| install_error(EXTRACT_FAILED))?;
    if !status.success() {
        let _ = fs::remove_dir_all(&staging);
        return Err(install_error(EXTRACT_FAILED));
    }
    let _ = fs::remove_dir_all(&destination);
    fs::rename(&staging, &destination).map_err(|_| install_error(EXTRACT_FAILED))?;
    Ok(())
}

/// The binary must exist and actually run: `--version` output has to name the
/// pinned version, otherwise the install is treated as corrupt.
pub(super) async fn verify_binary(app_data: &Path) -> AppResult<PathBuf> {
    let binary = executable_path(app_data);
    if !binary.is_file() {
        return Err(install_error(BINARY_FAILED));
    }
    let output = tokio::process::Command::new(&binary)
        .arg("--version")
        .output()
        .await
        .map_err(|_| install_error(BINARY_FAILED))?;
    let text = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !text.contains(CFT_VERSION) {
        return Err(install_error(BINARY_FAILED));
    }
    Ok(binary)
}

/// Wait for Chrome to publish its ephemeral CDP port, then confirm the
/// endpoint answers `/json/version`. Loopback-only by construction.
pub(super) async fn await_cdp(profile: &Path, deadline: Duration) -> AppResult<u16> {
    let marker = profile.join("DevToolsActivePort");
    let started = tokio::time::Instant::now();
    let port = loop {
        if let Ok(content) = fs::read_to_string(&marker) {
            if let Some(port) = parse_devtools_port(&content) {
                break port;
            }
        }
        if started.elapsed() > deadline {
            return Err(install_error(LAUNCH_FAILED));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| install_error(LAUNCH_FAILED))?;
    for _ in 0..20 {
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/json/version"))
            .send()
            .await
        {
            if response.status().is_success() {
                return Ok(port);
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(install_error(LAUNCH_FAILED))
}

// ---------------------------------------------------------------------------
// Tests — pure Rust; no downloads, no spawns, no network.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_manifest_is_exactly_the_verified_build() {
        assert_eq!(CFT_VERSION, "151.0.7922.34");
        assert!(CFT_URL.starts_with("https://cdn.playwright.dev/builds/cft/151.0.7922.34/"));
        assert_eq!(CFT_SHA256.len(), 64);
        assert!(CFT_SHA256.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(CFT_EXECUTABLE_RELATIVE.ends_with("Google Chrome for Testing"));
    }

    /// The real browser is headful on the user's own IP: the argv must pin
    /// the owned profile first, request an ephemeral loopback CDP port, and
    /// must never contain headless or proxy flags.
    #[test]
    fn argv_is_headful_ephemeral_cdp_and_proxyless() {
        let profile = PathBuf::from("/data/real-browser/profiles/default");
        let argv = build_argv(&profile, Some("https://example.com/"), false);
        assert_eq!(
            argv[0],
            "--user-data-dir=/data/real-browser/profiles/default",
            "profile pin must be first so nothing overrides it"
        );
        assert!(argv.iter().any(|a| a == "--remote-debugging-port=0"));
        assert!(argv.iter().any(|a| a == "--no-first-run"));
        assert!(argv.iter().any(|a| a == "--no-default-browser-check"));
        for forbidden in ["--headless", "--proxy-server", "--proxy-pac-url", "--remote-debugging-pipe"] {
            assert!(
                !argv.iter().any(|a| a.starts_with(forbidden)),
                "forbidden flag present: {forbidden}"
            );
        }
        assert_eq!(argv.last().unwrap(), "https://example.com/");
        let bare = build_argv(&profile, None, false);
        assert!(!bare.iter().any(|a| a.starts_with("http")));
        assert!(!bare.iter().any(|a| a.starts_with("--window-position")));
    }

    /// Embedded mode is the same headful browser with the window parked far
    /// offscreen — placement flags only, never `--headless` or emulation.
    #[test]
    fn embedded_argv_is_offscreen_headful_not_headless() {
        let profile = PathBuf::from("/data/real-browser/profiles/default");
        let argv = build_argv(&profile, Some("https://example.com/"), true);
        assert!(argv.iter().any(|a| a == EMBEDDED_POSITION_FLAG));
        assert!(argv.iter().any(|a| a == EMBEDDED_SIZE_FLAG));
        assert!(argv.iter().any(|a| a == EMBEDDED_OCCLUSION_FLAG));
        assert_eq!(EMBEDDED_POSITION_FLAG, "--window-position=20000,20000");
        assert_eq!(EMBEDDED_OCCLUSION_FLAG, "--disable-backgrounding-occluded-windows");
        for forbidden in ["--headless", "--proxy-server", "--headless=new"] {
            assert!(
                !argv.iter().any(|a| a.starts_with(forbidden)),
                "forbidden flag present: {forbidden}"
            );
        }
        // Everything else stays identical to the pop-out launch.
        let external = build_argv(&profile, Some("https://example.com/"), false);
        let embedded_only = [EMBEDDED_POSITION_FLAG, EMBEDDED_SIZE_FLAG, EMBEDDED_OCCLUSION_FLAG];
        let filtered: Vec<&String> = argv
            .iter()
            .filter(|a| !embedded_only.contains(&a.as_str()))
            .collect();
        assert_eq!(filtered, external.iter().collect::<Vec<_>>());
    }

    #[test]
    fn devtools_port_parses_first_line_only() {
        assert_eq!(parse_devtools_port("39251\n/devtools/browser/abc"), Some(39251));
        assert_eq!(parse_devtools_port("0\n"), None);
        assert_eq!(parse_devtools_port("not-a-port"), None);
        assert_eq!(parse_devtools_port(""), None);
    }

    #[test]
    fn owned_paths_stay_inside_app_data() {
        let app_data = Path::new("/tmp/appdata");
        for path in [
            install_root(app_data),
            executable_path(app_data),
            profile_dir(app_data),
            zip_path(app_data),
        ] {
            assert!(path.starts_with("/tmp/appdata/real-browser"), "{path:?}");
        }
        assert!(profile_dir(app_data).ends_with("profiles/default"));
        assert!(install_root(app_data).ends_with("chromium-151"));
    }

    /// The CDP endpoint is only handed out when the browser runs AND the user
    /// enabled assistant driving. Every other combination is an honest,
    /// non-erroring "not available".
    #[test]
    fn session_payload_gates_on_running_and_enabled() {
        let live = session_payload(Some(9222), true);
        assert_eq!(live["available"], true);
        assert_eq!(live["cdpEndpoint"], "http://127.0.0.1:9222");
        for (port, enabled) in [(Some(9222), false), (None, false), (None, true)] {
            let denied = session_payload(port, enabled);
            assert_eq!(denied["available"], false, "{port:?} {enabled}");
            assert!(denied.get("cdpEndpoint").is_none());
            assert!(denied["detail"].as_str().unwrap().len() <= 200);
        }
        // The disabled-toggle reason is reported even while running, so the
        // model can honestly tell the user which switch to flip.
        assert_eq!(
            session_payload(Some(9222), false)["detail"],
            "The user has not enabled assistant browser driving."
        );
    }

    /// Lifecycle transitions are generation-fenced: a stale monitor or a
    /// stale failure cannot clobber a newer launch, and `live()` never
    /// reports a non-running phase.
    #[test]
    fn lifecycle_is_generation_fenced_and_single_flight() {
        let real = RealChromium::default();
        let status = real.snapshot(None);
        assert_eq!(status.phase, RealPhase::Idle);
        // `installed` now means "openable without download": true on hosts
        // with a system Chrome even before any first-use download.
        assert_eq!(status.installed, openable(None));
        assert!(status.error.is_none());
        assert!(real.live().is_none());

        let first = real.begin().expect("first claim succeeds");
        assert!(real.begin().is_none(), "single-flight while busy");
        real.progress(first, RealPhase::Downloading, Some(40));
        let status = real.snapshot(None);
        assert_eq!(status.phase, RealPhase::Downloading);
        assert_eq!(status.progress_percent, Some(40));
        real.running(first, std::process::id(), 9222);
        assert_eq!(real.live(), Some((std::process::id(), 9222)));

        // A relaunch claims a new generation; the old exit monitor is inert.
        let second = real.begin().expect("relaunch allowed after running");
        real.exited(first);
        assert_eq!(real.snapshot(None).phase, RealPhase::Running, "stale exit ignored");
        real.failed(second, LAUNCH_FAILED);
        let status = real.snapshot(None);
        assert_eq!(status.phase, RealPhase::Error);
        assert_eq!(status.error, Some(LAUNCH_FAILED));
        assert!(real.live().is_none());
        assert_eq!(real.generation(), second);
    }

    #[test]
    fn exit_and_terminate_clear_the_session() {
        let real = RealChromium::default();
        let generation = real.begin().unwrap();
        real.running(generation, std::process::id(), 9300);
        real.exited(generation);
        let status = real.snapshot(None);
        assert_eq!(status.phase, RealPhase::Exited);
        assert!(status.pid.is_none());
        assert!(!status.cdp_ready);
        assert!(real.live().is_none());
        // Terminate on a non-running state is a no-op (nothing to signal).
        real.terminate();
        assert_eq!(real.snapshot(None).phase, RealPhase::Exited);
    }

    #[test]
    fn status_serializes_camel_case_for_the_renderer() {
        let real = RealChromium::default();
        let json = serde_json::to_value(real.snapshot(None)).unwrap();
        assert_eq!(json["phase"], "idle");
        assert_eq!(json["cdpReady"], false);
        assert_eq!(json["version"], CFT_VERSION);
        assert!(json.get("progressPercent").is_some());
        assert!(json.get("progress_percent").is_none());
        assert!(json.get("binary").is_some());
    }

    /// Stealth addendum 2026-09-13: the launch surface must never carry the
    /// classic automation tells, in either mode, and the binary preference
    /// is the user's own Google Chrome (real userAgentData brands) with the
    /// bundled Chrome for Testing only as fallback.
    #[test]
    fn launch_flags_avoid_automation_tells_and_binary_prefers_system_chrome() {
        let profile = PathBuf::from("/data/real-browser/profiles/default");
        for embedded in [false, true] {
            let argv = build_argv(&profile, Some("https://example.com/"), embedded);
            for forbidden in [
                "--enable-automation",
                "--headless",
                "--load-extension",
                "--disable-extensions",
                "--remote-debugging-pipe",
                "--user-agent",
            ] {
                assert!(
                    !argv.iter().any(|a| a.starts_with(forbidden)),
                    "automation tell present ({embedded}): {forbidden}"
                );
            }
        }
        assert_eq!(
            SYSTEM_CHROME_BINARY,
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        );
        // Preference order: system Chrome outranks the pinned CFT install.
        if system_chrome_installed() {
            assert_eq!(binary_kind(None), Some(BINARY_SYSTEM));
            assert_eq!(
                resolve_binary(Path::new("/nonexistent")).as_deref(),
                Some(Path::new(SYSTEM_CHROME_BINARY))
            );
        } else {
            assert_eq!(binary_kind(Some(Path::new("/nonexistent"))), None);
            assert!(resolve_binary(Path::new("/nonexistent")).is_none());
        }
        assert!(!openable(Some(Path::new("/nonexistent"))) || system_chrome_installed());
    }
}
