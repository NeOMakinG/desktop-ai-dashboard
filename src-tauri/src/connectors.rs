//! Google OAuth 2.0 PKCE plumbing, gated behind a build-time client id.
//! No authorization codes or tokens are logged, emitted, or saved in SQLite.
mod grants;
mod keychain;
mod lifecycle;
mod reads;
pub use grants::{ConnectorGrant, EgressConsent, GrantRequest, ReadOperation, RuntimeReadContext};
pub use reads::{dispatch_runtime_read, validate_runtime_delivery, ConnectorReadResult};
mod loopback;
mod oauth;
mod store;

use crate::types::{AppError, AppResult};
use keychain::{ConnectorSecrets, OsSecrets};
use lifecycle::{AttemptStatus, Lifecycle};
use loopback::Loopback;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use store::ConnectorStore;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use zeroize::{Zeroize, Zeroizing};

pub const GOOGLE_CLIENT_ID: Option<&'static str> = option_env!("FORMA_GOOGLE_CLIENT_ID");
const CALLBACK_PATH: &str = "/oauth2/google/callback";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_SCOPES: usize = 8;
const ALLOWED_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/gmail.metadata",
    "https://www.googleapis.com/auth/gmail.readonly",
    "https://www.googleapis.com/auth/calendar.readonly",
];
// Least-required default: metadata only, not both metadata and whole-mailbox read.
const DEFAULT_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/gmail.metadata",
    "https://www.googleapis.com/auth/calendar.readonly",
];
const CHANGE_EVENT: &str = "connectors:changed";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorStatus {
    pub id: String,
    pub provider: String,
    pub scopes: Vec<String>,
    pub display_name: Option<String>,
    pub connected_at: String,
    pub expires_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    pub authorize_url: String,
    pub attempt_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub google_available: bool,
    pub disabled_reason: Option<&'static str>,
    pub default_scopes: Vec<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSnapshot {
    pub revision: u64,
    pub attempt: Option<AttemptStatus>,
    pub items: Vec<ConnectorStatus>,
    pub cleanup_ids: Vec<String>,
}

#[derive(Deserialize)]
struct StoredTokens {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    expires_at: Option<String>,
}
impl Drop for StoredTokens {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

pub struct ConnectorsCore {
    // Lock order: lifecycle -> store -> secrets. Never hold these over await.
    lifecycle: Mutex<Lifecycle>,
    store: Mutex<ConnectorStore>,
    secrets: Arc<dyn ConnectorSecrets>,
    client: reqwest::Client,
}

/// App-scoped rather than process-global. The coordinator passes core's selected
/// directory and debug-only test directory; no environment lookup or fallback.
pub struct ConnectorState {
    directory: Option<PathBuf>,
    test_directory: Option<PathBuf>,
    core: OnceLock<AppResult<Arc<ConnectorsCore>>>,
}
impl ConnectorState {
    pub fn new(directory: Option<PathBuf>, test_directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            test_directory,
            core: OnceLock::new(),
        }
    }
    fn core(&self) -> AppResult<Arc<ConnectorsCore>> {
        self.core
            .get_or_init(|| {
                let directory = self.directory.as_ref().ok_or_else(AppError::storage)?;
                // A supplied QA identity may never point at a different/production DB.
                if self
                    .test_directory
                    .as_ref()
                    .is_some_and(|test| test != directory)
                {
                    return Err(AppError::storage());
                }
                build_state(directory, self.test_directory.as_deref())
            })
            .as_ref()
            .map(Arc::clone)
            .map_err(Clone::clone)
    }
    pub fn allows_pending_callback(&self, url: &reqwest::Url) -> bool {
        let Some(Ok(core)) = self.core.get() else {
            return false;
        };
        core.lifecycle
            .lock()
            .map(|life| life.allows_callback(url))
            .unwrap_or(false)
    }
}

fn build_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|_| {
            AppError::new(
                "connector_network",
                "The connector network client could not start.",
            )
        })
}
fn build_state(
    directory: &std::path::Path,
    test_directory: Option<&std::path::Path>,
) -> AppResult<Arc<ConnectorsCore>> {
    Ok(Arc::new(ConnectorsCore {
        lifecycle: Mutex::new(Lifecycle::default()),
        store: Mutex::new(ConnectorStore::open(directory)?),
        secrets: Arc::new(OsSecrets::new(test_directory)),
        client: build_client()?,
    }))
}
fn core(app: &AppHandle) -> AppResult<Arc<ConnectorsCore>> {
    app.try_state::<ConnectorState>()
        .ok_or_else(AppError::storage)?
        .core()
}
fn lifecycle_guard(core: &ConnectorsCore) -> AppResult<std::sync::MutexGuard<'_, Lifecycle>> {
    core.lifecycle.lock().map_err(|_| AppError::storage())
}
fn store_guard(core: &ConnectorsCore) -> AppResult<std::sync::MutexGuard<'_, ConnectorStore>> {
    core.store.lock().map_err(|_| AppError::storage())
}
fn validate_id(id: &str) -> AppResult<()> {
    let parsed = uuid::Uuid::parse_str(id).map_err(|_| AppError::invalid())?;
    if parsed.is_nil() || parsed.to_string() != id {
        return Err(AppError::invalid());
    }
    Ok(())
}
fn validate_scopes(scopes: &[String]) -> AppResult<()> {
    if scopes.is_empty() || scopes.len() > MAX_SCOPES {
        return Err(AppError::invalid());
    }
    for (index, scope) in scopes.iter().enumerate() {
        if !ALLOWED_SCOPES.contains(&scope.as_str()) || scopes[..index].contains(scope) {
            return Err(AppError::new(
                "invalid_scope",
                "Requested scope is not permitted for this connector.",
            ));
        }
    }
    Ok(())
}
fn granted_scopes(
    scope: Option<&str>,
    requested: &[String],
    refresh: bool,
) -> AppResult<Vec<String>> {
    // Initial omission is ambiguous: do not manufacture a grant from a request.
    // OAuth refresh omission means unchanged scopes, already verified at connect.
    let scopes = match scope {
        Some(scope) => scope
            .split_ascii_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        None if refresh => requested.to_owned(),
        None => return Err(scope_error()),
    };
    validate_scopes(&scopes).map_err(|_| scope_error())?;
    if scopes.iter().any(|scope| !requested.contains(scope)) {
        return Err(scope_error());
    }
    Ok(scopes)
}
fn scope_error() -> AppError {
    AppError::new(
        "connector_scope",
        "Google did not confirm the requested permission grant. No connection was enabled.",
    )
}
fn client_id() -> AppResult<&'static str> {
    GOOGLE_CLIENT_ID.filter(|id| !id.trim().is_empty()).ok_or_else(|| AppError::new(
        "connector_disabled", "Google OAuth client not configured. A supported desktop client and consent setup are required."))
}
fn emit_change(app: &AppHandle) {
    // Invalidations contain no stale status; the renderer reads the current snapshot.
    let _ = app.emit_to(tauri::EventTarget::webview_window("main"), CHANGE_EVENT, ());
}
fn snapshot(core: &ConnectorsCore) -> AppResult<ConnectorSnapshot> {
    let mut life = lifecycle_guard(core)?;
    life.expire();
    let store = store_guard(core)?;
    Ok(ConnectorSnapshot {
        revision: life.revision,
        attempt: life.attempt.clone(),
        items: store.list()?,
        cleanup_ids: store.cleanup_ids()?,
    })
}

async fn send_token_request(
    core: &ConnectorsCore,
    form: &[(&str, &str)],
) -> AppResult<oauth::TokenResponse> {
    let mut response = core
        .client
        .post(oauth::TOKEN_ENDPOINT)
        .form(form)
        .send()
        .await
        .map_err(|_| AppError::new("connector_network", "Could not reach the sign-in provider."))?;
    if !response.status().is_success() {
        return Err(AppError::new(
            "connector_exchange",
            "The provider refused this sign-in or refresh. Try reconnecting.",
        ));
    }
    if response
        .content_length()
        .is_some_and(|len| len > oauth::MAX_RESPONSE_BYTES as u64)
    {
        return Err(response_error());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        AppError::new(
            "connector_network",
            "The sign-in response could not be read.",
        )
    })? {
        if bytes.len() + chunk.len() > oauth::MAX_RESPONSE_BYTES {
            return Err(response_error());
        }
        bytes.extend_from_slice(&chunk);
    }
    oauth::parse_token_response(&bytes)
}
fn response_error() -> AppError {
    AppError::new(
        "connector_response",
        "The provider returned an unsupported token response.",
    )
}
fn serialize_tokens(
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: &str,
) -> AppResult<Zeroizing<String>> {
    #[derive(Serialize)]
    struct Payload<'a> {
        access_token: &'a str,
        refresh_token: Option<&'a str>,
        expires_at: &'a str,
    }
    serde_json::to_string(&Payload {
        access_token,
        refresh_token,
        expires_at,
    })
    .map(Zeroizing::new)
    .map_err(|_| AppError::storage())
}
fn read_stored(core: &ConnectorsCore, id: &str) -> AppResult<StoredTokens> {
    let raw = core.secrets.read(id)?;
    let parsed: StoredTokens = serde_json::from_str(&raw).map_err(|_| credential_error())?;
    if parsed.access_token.is_empty() {
        return Err(credential_error());
    }
    Ok(parsed)
}
fn credential_error() -> AppError {
    AppError::new(
        "connector_credential",
        "Stored credentials were unreadable.",
    )
}
fn cleanup_error() -> AppError {
    AppError::new(
        "connector_cleanup",
        "Disconnected locally. Credential cleanup failed. Retry cleanup in Accounts.",
    )
}
// The row is already staged/disconnected, so cleanup failure remains retryable
// and cannot expose a connection as active after an app restart.
fn rollback_staged(
    core: &ConnectorsCore,
    store: &ConnectorStore,
    id: &str,
    original: AppError,
) -> AppError {
    if core.secrets.remove(id).is_err() {
        return cleanup_error();
    }
    if store.delete(id).is_err() {
        return AppError::new(
            "connector_cleanup",
            "Disconnected locally. Cleanup is incomplete. Retry cleanup in Accounts.",
        );
    }
    original
}
fn commit_new(
    core: &ConnectorsCore,
    attempt_id: &str,
    requested: &[String],
    tokens: oauth::TokenResponse,
) -> AppResult<ConnectorStatus> {
    let mut life = lifecycle_guard(core)?;
    life.check(attempt_id)?;
    let scopes = granted_scopes(tokens.scope.as_deref(), requested, false)?;
    let now = chrono::Utc::now();
    let status = ConnectorStatus {
        id: uuid::Uuid::new_v4().to_string(),
        provider: "google".to_owned(),
        scopes,
        display_name: None,
        connected_at: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        expires_at: oauth::expires_at_from_secs(now, tokens.expires_in.unwrap_or(3600)),
    };
    let payload = serialize_tokens(
        &tokens.access_token,
        tokens.refresh_token.as_deref(),
        &status.expires_at,
    )?;
    let store = store_guard(core)?;
    store.stage(&status)?;
    let result = core
        .secrets
        .write(&status.id, &payload)
        .and_then(|_| life.check(attempt_id))
        .and_then(|_| store.upsert(&status));
    if let Err(error) = result {
        return Err(rollback_staged(core, &store, &status.id, error));
    }
    life.committed(attempt_id);
    Ok(status)
}
fn finish_attempt(core: &ConnectorsCore, id: &str, result: AppResult<()>) {
    if let Ok(mut life) = lifecycle_guard(core) {
        life.finish(id, result);
    }
}

// A dropped/cancelled command future also releases refresh ownership.
struct RefreshLease {
    core: Arc<ConnectorsCore>,
    id: String,
    generation: String,
}
impl Drop for RefreshLease {
    fn drop(&mut self) {
        if let Ok(mut life) = lifecycle_guard(&self.core) {
            life.finish_refresh(&self.id, &self.generation);
        }
    }
}
fn begin_refresh(
    core: &Arc<ConnectorsCore>,
    id: &str,
) -> AppResult<(RefreshLease, Zeroizing<String>)> {
    begin_refresh_checked(core, id, None)
}
fn begin_refresh_checked(
    core: &Arc<ConnectorsCore>,
    id: &str,
    read: Option<(&RuntimeReadContext, &str)>,
) -> AppResult<(RefreshLease, Zeroizing<String>)> {
    let mut life = lifecycle_guard(core)?;
    if let Some((context, lease_id)) = read {
        if context.connection_id != id {
            return Err(grants::denied());
        }
        life.grants
            .check_read(context, lease_id, chrono::Utc::now().timestamp_millis())?;
    }
    if store_guard(core)?.get(id)?.is_none() {
        return Err(AppError::new("not_found", "That connector was not found."));
    }
    let stored = read_stored(core, id)?;
    let refresh = Zeroizing::new(
        stored
            .refresh_token
            .as_ref()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| {
                AppError::new(
                    "connector_credential",
                    "No refresh token is stored. Reconnect this account.",
                )
            })?
            .clone(),
    );
    let generation = life.begin_refresh(id)?;
    Ok((
        RefreshLease {
            core: Arc::clone(core),
            id: id.to_owned(),
            generation,
        },
        refresh,
    ))
}
fn commit_refresh(
    lease: &RefreshLease,
    previous_refresh: &str,
    tokens: oauth::TokenResponse,
) -> AppResult<ConnectorStatus> {
    commit_refresh_checked(lease, previous_refresh, tokens, None)
}
fn commit_refresh_checked(
    lease: &RefreshLease,
    previous_refresh: &str,
    tokens: oauth::TokenResponse,
    read: Option<(&RuntimeReadContext, &str)>,
) -> AppResult<ConnectorStatus> {
    let core = &lease.core;
    let mut life = lifecycle_guard(core)?;
    if let Some((context, lease_id)) = read {
        if context.connection_id != lease.id {
            return Err(grants::denied());
        }
        life.grants
            .check_read(context, lease_id, chrono::Utc::now().timestamp_millis())?;
    }
    life.check_refresh(&lease.id, &lease.generation)?;
    let store = store_guard(core)?;
    let mut status = store.get(&lease.id)?.ok_or_else(lifecycle::stale_error)?;
    let previous_scopes = status.scopes.clone();
    status.scopes = match granted_scopes(tokens.scope.as_deref(), &status.scopes, true) {
        Ok(scopes) => scopes,
        Err(error) => {
            life.grants.revoke_connection(&status.id);
            store.mark_disconnected(&status.id)?;
            life.revision += 1;
            return Err(rollback_staged(core, &store, &status.id, error));
        }
    };
    if previous_scopes != status.scopes {
        life.grants.revoke_connection(&status.id);
    }
    status.expires_at =
        oauth::expires_at_from_secs(chrono::Utc::now(), tokens.expires_in.unwrap_or(3600));
    let payload = serialize_tokens(
        &tokens.access_token,
        Some(tokens.refresh_token.as_deref().unwrap_or(previous_refresh)),
        &status.expires_at,
    )?;
    // Same mutation lock covers generation check, durable revocation and writes.
    store.stage(&status)?;
    let result = core
        .secrets
        .write(&status.id, &payload)
        .and_then(|_| store.upsert(&status));
    life.revision += 1;
    if let Err(error) = result {
        life.grants.revoke_connection(&status.id);
        return Err(rollback_staged(core, &store, &status.id, error));
    }
    life.finish_refresh(&lease.id, &lease.generation);
    Ok(status)
}
fn disconnect_impl(core: &ConnectorsCore, id: &str) -> AppResult<()> {
    let mut life = lifecycle_guard(core)?;
    life.disconnect(id); // invalidate in-flight result even if storage subsequently fails
    let store = store_guard(core)?;
    let found = store.mark_disconnected(id).map_err(|_| {
        AppError::new(
            "connector_disconnect",
            "Disconnect failed. Connection status is unconfirmed.",
        )
    })?;
    core.secrets.remove(id).map_err(|_| {
        if found {
            cleanup_error()
        } else {
            credential_error()
        }
    })?;
    store.delete(id).map_err(|_| {
        AppError::new(
            "connector_cleanup",
            "Disconnected locally. Cleanup is incomplete. Retry cleanup in Accounts.",
        )
    })?;
    if !found {
        return Err(AppError::new("not_found", "That connector was not found."));
    }
    Ok(())
}

#[tauri::command]
pub async fn connectors_list(
    window: WebviewWindow,
    app: AppHandle,
) -> AppResult<Vec<ConnectorStatus>> {
    crate::first_party(&window)?;
    let core = core(&app)?;
    Ok(snapshot(&core)?.items)
}
#[tauri::command]
pub async fn connectors_status(
    window: WebviewWindow,
    app: AppHandle,
) -> AppResult<ConnectorSnapshot> {
    crate::first_party(&window)?;
    let core = core(&app)?;
    snapshot(&core)
}
#[tauri::command]
pub async fn connectors_capabilities(
    window: WebviewWindow,
    _app: AppHandle,
) -> AppResult<Capabilities> {
    crate::first_party(&window)?;
    let available = client_id().is_ok();
    Ok(Capabilities {
        google_available: available,
        disabled_reason: if available {
            None
        } else {
            Some("Google OAuth client not configured. A supported desktop client and consent setup are required.")
        },
        default_scopes: DEFAULT_SCOPES.to_vec(),
    })
}
#[tauri::command]
pub async fn connectors_cancel(
    window: WebviewWindow,
    app: AppHandle,
    attempt_id: String,
) -> AppResult<()> {
    crate::first_party(&window)?;
    validate_id(&attempt_id)?;
    let core = core(&app)?;
    lifecycle_guard(&core)?.cancel(&attempt_id)?;
    emit_change(&app);
    Ok(())
}
#[tauri::command]
pub async fn connectors_start_google(
    window: WebviewWindow,
    app: AppHandle,
    scopes: Vec<String>,
) -> AppResult<StartResponse> {
    crate::first_party(&window)?;
    let client_id = client_id()?;
    validate_scopes(&scopes)?;
    let core = core(&app)?;
    let loopback = Loopback::bind()?;
    let redirect = loopback.redirect_uri(CALLBACK_PATH);
    let pkce = oauth::pkce_pair();
    let state = Zeroizing::new(oauth::generate_state());
    let (id, cancel, deadline) =
        lifecycle_guard(&core)?.begin(loopback.port(), (*state).clone(), CALLBACK_TIMEOUT)?;
    let authorize_url =
        oauth::authorize_url(client_id, &redirect, &scopes, &state, &pkce.challenge);
    let attempt_id = id.clone();
    let app_handle = app.clone();
    let thread_core = Arc::clone(&core);
    let spawn = std::thread::Builder::new().name("google-callback".into()).spawn(move || {
        let callback = loopback.wait_for_callback(&state, CALLBACK_PATH, deadline, &cancel);
        let callback = match callback {
            Ok(callback) => callback,
            Err(error) => { finish_attempt(&thread_core, &id, Err(error)); drop(loopback); emit_change(&app_handle); return; }
        };
        let transition = lifecycle_guard(&thread_core).and_then(|mut life| life.exchanging(&id));
        if let Err(error) = transition { finish_attempt(&thread_core, &id, Err(error)); drop(loopback); emit_change(&app_handle); return; }
        // Revoke browser admission before releasing the bound socket.
        drop(loopback);
        emit_change(&app_handle);
        tauri::async_runtime::spawn(async move {
            let form = [("grant_type", "authorization_code"), ("client_id", client_id), ("redirect_uri", redirect.as_str()), ("code", callback.code.as_str()), ("code_verifier", pkce.verifier.as_str())];
            let result = tokio::select! {
                _ = cancel.cancelled() => Err(lifecycle::stale_error()),
                _ = tokio::time::sleep(deadline.saturating_duration_since(std::time::Instant::now())) => Err(lifecycle::timeout_error()),
                result = send_token_request(&thread_core, &form) => result,
            };
            let result = result.and_then(|tokens| commit_new(&thread_core, &id, &scopes, tokens)).map(|_| ());
            finish_attempt(&thread_core, &id, result);
            emit_change(&app_handle);
        });
    });
    if spawn.is_err() {
        let error = AppError::new(
            "connector_loopback",
            "The temporary sign-in callback listener is unavailable.",
        );
        finish_attempt(&core, &attempt_id, Err(error.clone()));
        emit_change(&app);
        return Err(error);
    }
    emit_change(&app);
    Ok(StartResponse {
        authorize_url,
        attempt_id,
    })
}
#[tauri::command]
pub async fn connectors_refresh(
    window: WebviewWindow,
    app: AppHandle,
    id: String,
) -> AppResult<ConnectorStatus> {
    crate::first_party(&window)?;
    validate_id(&id)?;
    let client_id = client_id()?;
    let core = core(&app)?;
    let (lease, refresh) = begin_refresh(&core, &id)?;
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh.as_str()),
    ];
    let result = send_token_request(&core, &form)
        .await
        .and_then(|tokens| commit_refresh(&lease, &refresh, tokens));
    drop(lease);
    emit_change(&app);
    result
}
#[tauri::command]
pub async fn connectors_disconnect(
    window: WebviewWindow,
    app: AppHandle,
    id: String,
) -> AppResult<()> {
    crate::first_party(&window)?;
    validate_id(&id)?;
    let core = core(&app)?;
    let result = disconnect_impl(&core, &id);
    emit_change(&app); // cleanup failures also change the durable local status
    result
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadCapabilities {
    live_google: bool,
    registration_available: bool,
    reasons: Vec<&'static str>,
    max_grant_seconds: i64,
    max_read_seconds: i64,
    max_window_days: u8,
    max_items: u16,
}
#[tauri::command]
pub async fn connectors_read_capabilities(window: WebviewWindow) -> AppResult<ReadCapabilities> {
    crate::first_party(&window)?;
    let registered = client_id().is_ok();
    let mut reasons = Vec::new();
    if !registered {
        reasons.push("Google OAuth client not configured. A registered desktop client and supported consent setup are required.");
    }
    reasons.push("Google compliance and account-read authorization have not been verified for this integration.");
    reasons.push("Runtime/model retention and revocation of account-derived prompts, history, snapshots and Interface content are not verified. Live egress is blocked.");
    Ok(ReadCapabilities {
        live_google: false,
        registration_available: registered,
        reasons,
        max_grant_seconds: grants::MAX_GRANT_SECONDS,
        max_read_seconds: grants::MAX_READ_SECONDS,
        max_window_days: 7,
        max_items: 100,
    })
}
#[tauri::command]
pub async fn connectors_grants_list(
    window: WebviewWindow,
    app: AppHandle,
) -> AppResult<Vec<ConnectorGrant>> {
    crate::first_party(&window)?;
    let core = core(&app)?;
    let mut life = lifecycle_guard(&core)?;
    Ok(life.grants.list(chrono::Utc::now().timestamp_millis()))
}
#[tauri::command]
pub async fn connectors_grant_register(
    window: WebviewWindow,
    app: AppHandle,
    request: GrantRequest,
) -> AppResult<ConnectorGrant> {
    crate::first_party(&window)?;
    client_id()?;
    let core = core(&app)?;
    let mut life = lifecycle_guard(&core)?;
    let status = store_guard(&core)?
        .get(&request.connection_id)?
        .ok_or_else(grants::denied)?;
    let grant = life
        .grants
        .register(request, &status, chrono::Utc::now().timestamp_millis())?;
    life.revision += 1;
    drop(life);
    emit_change(&app);
    Ok(grant)
}
#[tauri::command]
pub async fn connectors_grant_revoke(
    window: WebviewWindow,
    app: AppHandle,
    grant_id: String,
) -> AppResult<()> {
    crate::first_party(&window)?;
    validate_id(&grant_id)?;
    let core = core(&app)?;
    let mut life = lifecycle_guard(&core)?;
    life.grants.revoke(&grant_id);
    life.revision += 1;
    drop(life);
    emit_change(&app);
    Ok(())
}
/// Native runtime calls this before remote cancellation, including on config
/// changes and workspace deletion. No remote acknowledgment is needed to fence.
pub fn cancel_runtime_run(app: &AppHandle, workspace_id: &str, run_id: &str) -> AppResult<()> {
    let core = core(app)?;
    let mut life = lifecycle_guard(&core)?;
    life.grants
        .cancel_run(workspace_id, run_id, chrono::Utc::now().timestamp_millis())
}
#[tauri::command]
pub async fn connectors_run_cancel(
    window: WebviewWindow,
    app: AppHandle,
    workspace_id: String,
    run_id: String,
) -> AppResult<()> {
    crate::first_party(&window)?;
    cancel_runtime_run(&app, &workspace_id, &run_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use keychain::tests::MemorySecrets;
    use std::sync::atomic::{AtomicBool, Ordering};
    fn core_with(secrets: Arc<dyn ConnectorSecrets>) -> Arc<ConnectorsCore> {
        Arc::new(ConnectorsCore {
            lifecycle: Mutex::new(Lifecycle::default()),
            store: Mutex::new(store::tests::memory()),
            secrets,
            client: build_client().unwrap(),
        })
    }
    fn scopes() -> Vec<String> {
        DEFAULT_SCOPES.iter().map(|s| (*s).to_owned()).collect()
    }
    fn tokens(scope: Option<&str>) -> oauth::TokenResponse {
        oauth::TokenResponse {
            access_token: "synthetic-access".into(),
            refresh_token: Some("synthetic-refresh".into()),
            expires_in: Some(3600),
            scope: scope.map(str::to_owned),
            token_type: Some("Bearer".into()),
        }
    }
    fn seed(core: &ConnectorsCore) -> ConnectorStatus {
        let (id, _, _) = lifecycle_guard(core)
            .unwrap()
            .begin(12345, "synthetic-state".into(), CALLBACK_TIMEOUT)
            .unwrap();
        commit_new(core, &id, &scopes(), tokens(Some(&scopes().join(" ")))).unwrap()
    }
    #[test]
    fn scope_validation_rejects_missing_wide_and_duplicate_grants() {
        assert!(validate_scopes(&[]).is_err());
        assert!(validate_scopes(&["gmail.send".into()]).is_err());
        assert!(validate_scopes(&vec![ALLOWED_SCOPES[0].into(); MAX_SCOPES + 1]).is_err());
        assert!(validate_scopes(&[ALLOWED_SCOPES[0].into(), ALLOWED_SCOPES[0].into()]).is_err());
        assert!(granted_scopes(None, &scopes(), false).is_err());
        assert!(granted_scopes(Some(""), &scopes(), false).is_err());
        assert!(granted_scopes(Some(ALLOWED_SCOPES[1]), &scopes(), false).is_err());
        assert_eq!(
            granted_scopes(Some(ALLOWED_SCOPES[0]), &scopes(), false).unwrap(),
            vec![ALLOWED_SCOPES[0]]
        );
        assert_eq!(granted_scopes(None, &scopes(), true).unwrap(), scopes());
    }
    #[test]
    fn id_validation_rejects_nil_malformed_and_noncanonical() {
        for id in [
            "not-a-uuid",
            "00000000-0000-0000-0000-000000000000",
            "00000000000000000000000000000001",
        ] {
            assert!(validate_id(id).is_err());
        }
        assert!(validate_id(&uuid::Uuid::new_v4().to_string()).is_ok());
    }
    #[test]
    fn actual_partial_grant_is_persisted_and_tokens_never_enter_snapshot() {
        let secrets = Arc::new(MemorySecrets::default());
        let core = core_with(secrets.clone());
        let (id, _, _) = lifecycle_guard(&core)
            .unwrap()
            .begin(12345, "synthetic-state".into(), CALLBACK_TIMEOUT)
            .unwrap();
        let status = commit_new(&core, &id, &scopes(), tokens(Some(ALLOWED_SCOPES[0]))).unwrap();
        assert_eq!(status.scopes, vec![ALLOWED_SCOPES[0]]);
        let json = serde_json::to_string(&snapshot(&core).unwrap()).unwrap();
        for secret in ["synthetic-state", "synthetic-access", "synthetic-refresh"] {
            assert!(!json.contains(secret));
        }
        assert!(json.contains("connected"));
        assert!(commit_new(&core, &id, &scopes(), tokens(Some(ALLOWED_SCOPES[0]))).is_err());
        assert_eq!(secrets.0.lock().unwrap().len(), 1);
    }
    #[test]
    fn cancelled_and_scope_mismatch_exchanges_never_write_credentials() {
        let secrets = Arc::new(MemorySecrets::default());
        let core = core_with(secrets.clone());
        let (id, _, _) = lifecycle_guard(&core)
            .unwrap()
            .begin(12345, "state".into(), CALLBACK_TIMEOUT)
            .unwrap();
        assert!(commit_new(&core, &id, &scopes(), tokens(Some(ALLOWED_SCOPES[1]))).is_err());
        lifecycle_guard(&core).unwrap().cancel(&id).unwrap();
        assert!(commit_new(&core, &id, &scopes(), tokens(Some(ALLOWED_SCOPES[0]))).is_err());
        assert!(secrets.0.lock().unwrap().is_empty());
        assert!(snapshot(&core).unwrap().items.is_empty());
    }
    #[test]
    fn disconnect_during_refresh_cannot_resurrect_tokens_or_connection() {
        let secrets = Arc::new(MemorySecrets::default());
        let core = core_with(secrets.clone());
        let status = seed(&core);
        let (lease, refresh) = begin_refresh(&core, &status.id).unwrap();
        disconnect_impl(&core, &status.id).unwrap();
        assert_eq!(
            commit_refresh(&lease, &refresh, tokens(None))
                .unwrap_err()
                .code,
            "connector_stale"
        );
        assert!(secrets.0.lock().unwrap().is_empty());
        assert!(snapshot(&core).unwrap().items.is_empty());
        let next = seed(&core);
        assert!(commit_refresh(&lease, &refresh, tokens(None)).is_err());
        assert_eq!(snapshot(&core).unwrap().items, vec![next]);
    }
    #[test]
    fn refresh_is_single_flight_and_preserves_or_narrows_verified_grants() {
        let core = core_with(Arc::new(MemorySecrets::default()));
        let status = seed(&core);
        let (lease, refresh) = begin_refresh(&core, &status.id).unwrap();
        assert!(begin_refresh(&core, &status.id).is_err());
        let updated = commit_refresh(&lease, &refresh, tokens(Some(ALLOWED_SCOPES[0]))).unwrap();
        assert_eq!(updated.scopes, vec![ALLOWED_SCOPES[0]]);
        drop(lease);
        let (lease, refresh) = begin_refresh(&core, &status.id).unwrap();
        assert_eq!(
            commit_refresh(&lease, &refresh, tokens(None))
                .unwrap()
                .scopes,
            updated.scopes
        );
    }
    #[test]
    fn failed_or_dropped_refresh_releases_lease_without_writes() {
        let core = core_with(Arc::new(MemorySecrets::default()));
        let status = seed(&core);
        let (lease, _) = begin_refresh(&core, &status.id).unwrap();
        drop(lease); // same path as exchange error/cancelled IPC future
        assert!(begin_refresh(&core, &status.id).is_ok());
        assert_eq!(snapshot(&core).unwrap().items, vec![status]);
    }
    #[derive(Default)]
    struct FailingSecrets {
        memory: MemorySecrets,
        fail_remove: AtomicBool,
        fail_write: AtomicBool,
    }
    impl ConnectorSecrets for FailingSecrets {
        fn read(&self, id: &str) -> AppResult<Zeroizing<String>> {
            self.memory.read(id)
        }
        fn write(&self, id: &str, secret: &str) -> AppResult<()> {
            if self.fail_write.load(Ordering::SeqCst) {
                return Err(credential_error());
            }
            self.memory.write(id, secret)
        }
        fn remove(&self, id: &str) -> AppResult<()> {
            if self.fail_remove.load(Ordering::SeqCst) {
                return Err(credential_error());
            }
            self.memory.remove(id)
        }
    }
    #[test]
    fn cleanup_failure_revokes_durably_and_remains_retryable() {
        let secrets = Arc::new(FailingSecrets::default());
        let core = core_with(secrets.clone());
        let status = seed(&core);
        let (lease, refresh) = begin_refresh(&core, &status.id).unwrap();
        secrets.fail_remove.store(true, Ordering::SeqCst);
        assert_eq!(
            disconnect_impl(&core, &status.id).unwrap_err().code,
            "connector_cleanup"
        );
        assert!(snapshot(&core).unwrap().items.is_empty());
        assert_eq!(
            snapshot(&core).unwrap().cleanup_ids,
            vec![status.id.clone()]
        );
        assert!(begin_refresh(&core, &status.id).is_err());
        assert!(commit_refresh(&lease, &refresh, tokens(None)).is_err());
        secrets.fail_remove.store(false, Ordering::SeqCst);
        disconnect_impl(&core, &status.id).unwrap();
        assert!(snapshot(&core).unwrap().cleanup_ids.is_empty());
        assert!(secrets.memory.0.lock().unwrap().is_empty());
    }
    #[test]
    fn secret_write_failure_never_publishes_connected_and_tracks_cleanup_failure() {
        let secrets = Arc::new(FailingSecrets::default());
        let core = core_with(secrets.clone());
        secrets.fail_write.store(true, Ordering::SeqCst);
        secrets.fail_remove.store(true, Ordering::SeqCst);
        let (id, _, _) = lifecycle_guard(&core)
            .unwrap()
            .begin(12345, "state".into(), CALLBACK_TIMEOUT)
            .unwrap();
        let error = commit_new(&core, &id, &scopes(), tokens(Some(ALLOWED_SCOPES[0]))).unwrap_err();
        assert_eq!(error.code, "connector_cleanup");
        finish_attempt(&core, &id, Err(error));
        let snapshot = snapshot(&core).unwrap();
        assert!(snapshot.items.is_empty());
        assert_eq!(snapshot.cleanup_ids.len(), 1);
        assert_eq!(
            snapshot.attempt.unwrap().phase,
            lifecycle::AttemptPhase::Failed
        );
    }
    #[test]
    fn stored_tokens_round_trip_and_reject_malformed() {
        let core = core_with(Arc::new(MemorySecrets::default()));
        for raw in ["{}", "not-json", r#"{"access_token":""}"#] {
            core.secrets.write("id", raw).unwrap();
            assert!(read_stored(&core, "id").is_err());
        }
        let payload = serialize_tokens("aaa", Some("rrr"), "2026-01-01T00:00:00.000Z").unwrap();
        core.secrets.write("id", &payload).unwrap();
        let parsed = read_stored(&core, "id").unwrap();
        assert_eq!(parsed.access_token, "aaa");
        assert_eq!(parsed.refresh_token.as_deref(), Some("rrr"));
        assert_eq!(
            parsed.expires_at.as_deref(),
            Some("2026-01-01T00:00:00.000Z")
        );
    }
    #[test]
    fn refresh_grant_mismatch_disables_previous_scope_claim() {
        let core = core_with(Arc::new(MemorySecrets::default()));
        let status = seed(&core);
        let (lease, refresh) = begin_refresh(&core, &status.id).unwrap();
        let error = commit_refresh(&lease, &refresh, tokens(Some(ALLOWED_SCOPES[1]))).unwrap_err();
        assert_eq!(error.code, "connector_scope");
        assert!(snapshot(&core).unwrap().items.is_empty());
        assert!(core.secrets.read(&status.id).is_err());
    }

    #[test]
    fn exchange_failures_are_visible_and_revoke_callback_capability() {
        let core = core_with(Arc::new(MemorySecrets::default()));
        for code in [
            "connector_network",
            "connector_exchange",
            "connector_response",
            "connector_denied",
            "connector_credential",
        ] {
            let (id, _, _) = lifecycle_guard(&core)
                .unwrap()
                .begin(12345, "fixture-state".into(), CALLBACK_TIMEOUT)
                .unwrap();
            finish_attempt(
                &core,
                &id,
                Err(AppError::new(code, "Sanitized fixture failure.")),
            );
            let status = snapshot(&core).unwrap();
            assert_eq!(status.attempt.unwrap().error.unwrap().code, code);
            assert!(status.items.is_empty());
            assert!(!lifecycle_guard(&core).unwrap().allows_callback(&reqwest::Url::parse("http://127.0.0.1:12345/oauth2/google/callback?state=fixture-state&code=fixture").unwrap()));
        }
    }

    #[test]
    fn distinct_qa_directories_have_distinct_databases_without_touching_keychain() {
        let root = std::env::temp_dir().join(format!(
            "forma-connector-isolation-{}",
            uuid::Uuid::new_v4()
        ));
        let first = root.join("qa-a");
        let second = root.join("qa-b");
        {
            let a = ConnectorState::new(Some(first.clone()), Some(first.clone()));
            let b = ConnectorState::new(Some(second.clone()), Some(second.clone()));
            // Seed content-free metadata only, never call the OS secret implementation.
            let a_core = a.core().unwrap();
            let metadata = ConnectorStatus {
                id: uuid::Uuid::new_v4().to_string(),
                provider: "google".into(),
                scopes: scopes(),
                display_name: None,
                connected_at: "fixture-start".into(),
                expires_at: "fixture-end".into(),
            };
            store_guard(&a_core).unwrap().stage(&metadata).unwrap();
            assert_eq!(snapshot(&a_core).unwrap().cleanup_ids, vec![metadata.id]);
            assert!(snapshot(&b.core().unwrap()).unwrap().cleanup_ids.is_empty());
            assert!(first.join("connectors.sqlite3").is_file());
            assert!(second.join("connectors.sqlite3").is_file());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn qa_core_rejects_mismatched_storage_and_callback_check_never_opens_storage() {
        let state = ConnectorState::new(
            Some(PathBuf::from("/unused-production")),
            Some(PathBuf::from("/unused-qa")),
        );
        assert!(!state.allows_pending_callback(
            &reqwest::Url::parse("http://127.0.0.1:12345/oauth2/google/callback?state=ok").unwrap()
        ));
        assert!(state.core.get().is_none());
        assert!(state.core().is_err());
    }
}
