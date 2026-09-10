//! Private OAuth-only system-browser launcher. Never reads browser profiles.
use crate::types::{AppError, AppResult};

pub fn supported() -> bool {
    cfg!(target_os = "macos")
}

pub fn open_error() -> AppError {
    AppError::new(
        "connector_browser",
        "Could not open Google sign-in in your default browser. Sign-in stopped; try again.",
    )
}

// Spawn synchronously so the caller can fence it with the pending-attempt lock.
// Waiting is asynchronous and bounded by the caller. No shell or inherited output.
#[cfg(target_os = "macos")]
pub fn spawn(url: &str) -> AppResult<impl std::future::Future<Output = AppResult<()>>> {
    use std::process::Stdio;
    let mut child = tokio::process::Command::new("/usr/bin/open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| open_error())?;
    Ok(async move {
        let status = child.wait().await.map_err(|_| open_error())?;
        if status.success() {
            Ok(())
        } else {
            Err(open_error())
        }
    })
}

#[cfg(not(target_os = "macos"))]
pub fn spawn(_url: &str) -> AppResult<std::future::Ready<AppResult<()>>> {
    Err(AppError::new(
        "connector_disabled",
        "Google system-browser sign-in is not supported on this platform yet.",
    ))
}
