//! Native Composio connector client (v3 REST, spike-verified shapes).
//!
//! Host-owned boundaries: the project API key stays in the OS keychain and is
//! never logged, persisted, or exposed to the renderer or the Python runtime;
//! every Composio response is treated as untrusted data with bounded sizes;
//! connection initiation reuses the system-browser opener and loopback listener
//! shared with the Google connector. Endpoint map:
//! .local/receipts/macos-completion-20260910/composio-implementation-spike.md
use super::keychain::{ComposioKeyStore, ComposioKeys};
use super::loopback::Loopback;
use crate::types::{AppError, AppResult};
use serde::Serialize;
use serde_json::{json, Value};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

const BASE_URL: &str = "https://backend.composio.dev/api/v3";
const DEFAULT_RELAY_URL: &str = "http://100.90.255.43:9231";
const CALLBACK_PATH: &str = "/composio/callback";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const LOCAL_USER_ID: &str = "forma-desktop";
const MAX_BODY_BYTES: usize = 512 * 1024;
const MAX_CATALOG_LIMIT: u16 = 200;
const MAX_PAGE_ITEMS: usize = 1_000;
const MAX_ACCOUNTS: usize = 100;
/// Single bound for both the fetch limit and the runtime tool projection (F5').
const MAX_LIST_ITEMS: usize = 50;
const MAX_PROMPTS: usize = 64;
const PROMPT_TTL: Duration = Duration::from_secs(600);
const CHANGE_EVENT: &str = "composio:changed";
const PROMPT_EVENT: &str = "composio:connect-prompt";
pub const RUNTIME_LIST_TOOL: &str = "forma_list_connected_services";
pub const RUNTIME_CONNECT_TOOL: &str = "forma_request_service_connection";

fn network_error() -> AppError {
    AppError::new(
        "composio_network",
        "Could not reach the Composio connector service. Check your connection and try again.",
    )
}
fn data_error() -> AppError {
    AppError::new(
        "composio_response",
        "The Composio connector service returned an unsupported response.",
    )
}
fn timeout_error() -> AppError {
    AppError::new(
        "composio_timeout",
        "Composio did not confirm this connection in time. Check the service on the Services page and retry.",
    )
}
fn stale_error() -> AppError {
    AppError::new(
        "composio_stale",
        "This connection attempt is no longer active.",
    )
}
fn browser_error() -> AppError {
    AppError::new(
        "composio_browser",
        "Could not open the service sign-in page in your default browser. Sign-in stopped; try again.",
    )
}
fn pending_error() -> AppError {
    AppError::new(
        "composio_pending",
        "A service connection is already in progress. Finish or cancel it first.",
    )
}

pub fn valid_service_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}
/// Composio identifiers (auth config / connected account nanoids).
fn valid_composio_id(value: &str) -> bool {
    !value.is_empty()
        && (8..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn sanitize_detail(value: Option<&str>, cap: usize) -> Option<String> {
    value
        .filter(|text| !text.is_empty())
        .map(|text| {
            text.chars()
                .filter(|c| !c.is_control())
                .take(cap)
                .collect::<String>()
        })
        .filter(|text| !text.is_empty())
}

// The v3 docs document snake_case fields; aliases tolerate camelCase variants.
fn camel(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for character in name.chars() {
        if character == '_' {
            upper = true;
        } else if upper {
            out.extend(character.to_uppercase());
            upper = false;
        } else {
            out.push(character);
        }
    }
    out
}
fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    value.get(name).or_else(|| value.get(&camel(name)))
}
fn bounded_str<'a>(value: &'a Value, name: &str, cap: usize) -> Option<&'a str> {
    field(value, name)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= cap)
}
fn safe_https_url(value: &str) -> bool {
    if value.len() > 2048 {
        return false;
    }
    match reqwest::Url::parse(value) {
        Ok(url) => {
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
        }
        Err(_) => false,
    }
}
fn composio_host_url(value: &str) -> bool {
    if !safe_https_url(value) {
        return false;
    }
    reqwest::Url::parse(value)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host == "composio.dev" || host.ends_with(".composio.dev"))
        })
        .unwrap_or(false)
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolkitSummary {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub logo: Option<String>,
    pub categories: Vec<String>,
    pub tools_count: Option<u64>,
    pub no_auth: bool,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub name: String,
    pub id: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPage {
    pub items: Vec<ToolkitSummary>,
    pub total_items: u64,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub id: String,
    pub service: String,
    pub status: &'static str,
    pub status_detail: Option<String>,
    pub alias: Option<String>,
    pub word_id: Option<String>,
    pub connected_at: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub key_configured: bool,
    pub relay: bool,
    pub attempt: Option<AttemptStatus>,
    pub items: Vec<ServiceStatus>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    pub attempt_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AttemptPhase {
    Pending,
    Confirming,
    Connected,
    Cancelled,
    Failed,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptStatus {
    pub id: String,
    pub service: String,
    pub connected_account_id: Option<String>,
    pub phase: AttemptPhase,
    pub expires_at: String,
    pub error: Option<AppError>,
    pub detail: Option<String>,
}

struct Attempt {
    status: AttemptStatus,
    deadline: Instant,
    cancel: CancellationToken,
}
struct PromptRecord {
    workspace_id: String,
    service: String,
    run_id: String,
    at: Instant,
}
struct Inner {
    revision: u64,
    /// Live single-flight slot; only a terminal transition may clear it.
    pending: Option<Attempt>,
    /// Last attempt status for snapshots; outlives the live slot.
    attempt_status: Option<AttemptStatus>,
    prompts: Vec<PromptRecord>,
}
pub struct ComposioCore {
    inner: Mutex<Inner>,
    keys: Arc<dyn ComposioKeys>,
    client: reqwest::Client,
    /// Founder relay mode (2026-09-13): when a relay token exists the app
    /// routes every Composio call through the Forma relay and the project
    /// API key never touches this machine. Relay wins over a stored key.
    relay: Option<(reqwest::Url, Arc<Zeroizing<String>>)>,
}
/// App-scoped rather than process-global; QA key storage is namespaced like
/// the Google connector secrets and never falls back to the production entry.
pub struct ComposioState {
    test_directory: Option<PathBuf>,
    core: OnceLock<AppResult<Arc<ComposioCore>>>,
}
impl ComposioState {
    pub fn new(test_directory: Option<PathBuf>) -> Self {
        Self {
            test_directory,
            core: OnceLock::new(),
        }
    }
    fn core(&self) -> AppResult<Arc<ComposioCore>> {
        self.core
            .get_or_init(|| {
                Ok(Arc::new(ComposioCore {
                    inner: Mutex::new(Inner {
                        revision: 0,
                        pending: None,
                        attempt_status: None,
                        prompts: Vec::new(),
                    }),
                    keys: Arc::new(ComposioKeyStore::new(self.test_directory.as_deref())),
                    client: build_client()?,
                    relay: if self.test_directory.is_none() {
                        production_relay()
                    } else {
                        None
                    },
                }))
            })
            .as_ref()
            .map(Arc::clone)
            .map_err(Clone::clone)
    }
}
fn production_relay() -> Option<(reqwest::Url, Arc<Zeroizing<String>>)> {
    let token = Arc::new(crate::connectors::keychain::relay_token()?);
    let base = std::env::var("FORMA_COMPOSIO_RELAY")
        .ok()
        .and_then(|value| reqwest::Url::parse(&value).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or_else(|| {
            reqwest::Url::parse(DEFAULT_RELAY_URL).expect("static relay URL parses")
        });
    Some((base, token))
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
                "composio_network",
                "The Composio connector network client could not start.",
            )
        })
}
fn core(app: &AppHandle) -> AppResult<Arc<ComposioCore>> {
    app.try_state::<ComposioState>()
        .ok_or_else(AppError::storage)?
        .core()
}
fn lock_inner(core: &ComposioCore) -> AppResult<std::sync::MutexGuard<'_, Inner>> {
    core.inner.lock().map_err(|_| AppError::storage())
}
fn emit_change(app: &AppHandle) {
    let _ = app.emit_to(tauri::EventTarget::webview_window("main"), CHANGE_EVENT, ());
}

impl ComposioCore {
    fn http(&self) -> AppResult<ComposioHttp> {
        if let Some((base, token)) = &self.relay {
            return Ok(ComposioHttp {
                client: self.client.clone(),
                key: Arc::new(Zeroizing::new(String::new())),
                relay_base: Some(base.clone()),
                relay_token: Some(Arc::clone(token)),
            });
        }
        let key = self.keys.read()?;
        Ok(ComposioHttp {
            client: self.client.clone(),
            key: Arc::new(key),
            relay_base: None,
            relay_token: None,
        })
    }
    /// Single-flight connection attempt; a terminal transition frees the slot
    /// (F1) so the next start can begin without an app restart.
    fn begin_attempt(
        &self,
        service: &str,
        timeout: Duration,
    ) -> AppResult<(String, CancellationToken)> {
        let mut inner = lock_inner(self)?;
        Self::expire_locked(&mut inner);
        if inner.pending.is_some() {
            return Err(pending_error());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        let status = AttemptStatus {
            id: id.clone(),
            service: service.to_owned(),
            connected_account_id: None,
            phase: AttemptPhase::Pending,
            expires_at: chrono::Utc::now()
                .checked_add_signed(chrono::Duration::seconds(timeout.as_secs() as i64))
                .unwrap_or_else(chrono::Utc::now)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            error: None,
            detail: None,
        };
        inner.attempt_status = Some(status.clone());
        inner.pending = Some(Attempt {
            status,
            deadline: Instant::now() + timeout,
            cancel: cancel.clone(),
        });
        inner.revision += 1;
        Ok((id, cancel))
    }
    fn expire_locked(inner: &mut Inner) {
        if inner
            .pending
            .as_ref()
            .is_some_and(|attempt| Instant::now() >= attempt.deadline)
        {
            Self::terminal_locked(inner, AttemptPhase::Failed, Some(timeout_error()), None);
        }
    }
    /// Take the live slot, cancel it, and record the terminal status.
    fn terminal_locked(
        inner: &mut Inner,
        phase: AttemptPhase,
        error: Option<AppError>,
        detail: Option<String>,
    ) {
        if let Some(pending) = inner.pending.take() {
            pending.cancel.cancel();
            let id = pending.status.id.clone();
            if inner
                .attempt_status
                .as_ref()
                .is_some_and(|status| status.id == id)
            {
                let status = inner.attempt_status.as_mut().unwrap();
                status.phase = phase;
                status.error = error;
                status.detail = detail;
            }
            inner.revision += 1;
        }
    }
    fn mutate_attempt<F: FnOnce(&mut Attempt) -> bool>(&self, id: &str, mutate: F) -> bool {
        let Ok(mut inner) = lock_inner(self) else {
            return false;
        };
        Self::expire_locked(&mut inner);
        let Some(attempt) = inner.pending.as_mut() else {
            return false;
        };
        if attempt.status.id != id {
            return false;
        }
        let changed = mutate(attempt);
        if changed {
            let status = attempt.status.clone();
            inner.attempt_status = Some(status);
            inner.revision += 1;
        }
        changed
    }
    fn attach_account(&self, id: &str, connected_account_id: &str) -> bool {
        self.mutate_attempt(id, |attempt| {
            if attempt.status.phase == AttemptPhase::Pending {
                attempt.status.connected_account_id = Some(connected_account_id.to_owned());
                true
            } else {
                false
            }
        })
    }
    fn mark_confirming(&self, id: &str) -> bool {
        self.mutate_attempt(id, |attempt| {
            if attempt.status.phase == AttemptPhase::Pending {
                attempt.status.phase = AttemptPhase::Confirming;
                true
            } else {
                false
            }
        })
    }
    fn check_attempt(&self, id: &str) -> AppResult<CancellationToken> {
        let mut inner = lock_inner(self)?;
        Self::expire_locked(&mut inner);
        let attempt = inner.pending.as_ref().ok_or_else(stale_error)?;
        if attempt.status.id != id || attempt.cancel.is_cancelled() {
            return Err(stale_error());
        }
        Ok(attempt.cancel.clone())
    }
    fn finish_attempt(
        &self,
        id: &str,
        phase: AttemptPhase,
        error: Option<AppError>,
        detail: Option<String>,
    ) -> bool {
        let Ok(mut inner) = lock_inner(self) else {
            return false;
        };
        Self::expire_locked(&mut inner);
        if !inner
            .pending
            .as_ref()
            .is_some_and(|attempt| attempt.status.id == id)
        {
            return false;
        }
        Self::terminal_locked(&mut inner, phase, error, detail);
        true
    }
    fn cancel_attempt(&self, id: &str) -> AppResult<()> {
        let mut inner = lock_inner(self)?;
        Self::expire_locked(&mut inner);
        if !inner.pending.is_some() {
            return Err(AppError::new(
                "not_found",
                "That connection attempt was not found.",
            ));
        }
        if !inner
            .pending
            .as_ref()
            .is_some_and(|attempt| attempt.status.id == id)
        {
            return Err(stale_error());
        }
        Self::terminal_locked(&mut inner, AttemptPhase::Cancelled, None, None);
        Ok(())
    }
    fn snapshot_attempt(&self) -> Option<AttemptStatus> {
        let mut inner = lock_inner(self).ok()?;
        Self::expire_locked(&mut inner);
        inner.attempt_status.clone()
    }
    /// The live single-flight attempt only; terminal statuses do not count.
    fn live_attempt(&self) -> Option<AttemptStatus> {
        let mut inner = lock_inner(self).ok()?;
        Self::expire_locked(&mut inner);
        inner.pending.as_ref().map(|attempt| attempt.status.clone())
    }
    fn revision(&self) -> u64 {
        lock_inner(self).map(|inner| inner.revision).unwrap_or(0)
    }
    /// Hermes tool prompts are deduplicated per workspace+service with a TTL;
    /// rendering the card is a host decision, never message text. Suppression
    /// lifts on dismissal, disconnect, connection, or expiry (F3).
    fn should_prompt(&self, workspace_id: &str, service: &str, run_id: &str) -> bool {
        let Ok(mut inner) = lock_inner(self) else {
            return false;
        };
        let now = Instant::now();
        let expired = inner.prompts.iter().any(|record| {
            record.workspace_id == workspace_id
                && record.service == service
                && now >= record.at + PROMPT_TTL
        });
        if expired {
            inner.prompts.retain(|record| {
                !(record.workspace_id == workspace_id && record.service == service)
            });
        } else if let Some(record) = inner
            .prompts
            .iter_mut()
            .find(|record| record.workspace_id == workspace_id && record.service == service)
        {
            record.run_id = run_id.to_owned();
            return false;
        }
        if inner.prompts.len() >= MAX_PROMPTS {
            inner.prompts.remove(0);
        }
        inner.prompts.push(PromptRecord {
            workspace_id: workspace_id.to_owned(),
            service: service.to_owned(),
            run_id: run_id.to_owned(),
            at: now,
        });
        true
    }
    fn resolve_prompt_for_service(&self, service: &str) {
        if let Ok(mut inner) = lock_inner(self) {
            inner.prompts.retain(|record| record.service != service);
        }
    }
    fn resolve_all_prompts(&self) {
        if let Ok(mut inner) = lock_inner(self) {
            inner.prompts.clear();
        }
    }
    fn dismiss_prompt(&self, workspace_id: &str, service: &str) {
        if let Ok(mut inner) = lock_inner(self) {
            inner.prompts.retain(|record| {
                !(record.workspace_id == workspace_id && record.service == service)
            });
        }
    }
}

trait Transport: Send + Sync {
    fn request(
        &self,
        method: reqwest::Method,
        url: reqwest::Url,
        body: Option<String>,
    ) -> Pin<Box<dyn Future<Output = AppResult<Zeroizing<Vec<u8>>>> + Send + '_>>;
}

struct ComposioHttp {
    client: reqwest::Client,
    key: Arc<Zeroizing<String>>,
    relay_base: Option<reqwest::Url>,
    relay_token: Option<Arc<Zeroizing<String>>>,
}
impl ComposioHttp {
    fn key_header(
        &self,
    ) -> AppResult<(reqwest::header::HeaderValue, reqwest::header::HeaderValue)> {
        use reqwest::header::HeaderValue;
        let mut bearer = HeaderValue::from_str(&format!("Bearer {}", self.key.as_str()))
            .map_err(|_| AppError::invalid())?;
        bearer.set_sensitive(true);
        let mut api_key =
            HeaderValue::from_str(self.key.as_str()).map_err(|_| AppError::invalid())?;
        api_key.set_sensitive(true);
        Ok((bearer, api_key))
    }
}
fn map_status(status: reqwest::StatusCode) -> AppError {
    match status.as_u16() {
        401 | 403 => AppError::new(
            "composio_auth",
            "Composio rejected the stored API key. Update it in Settings.",
        ),
        404 => AppError::new(
            "composio_not_found",
            "Composio does not know that service or account.",
        ),
        410 => AppError::new(
            "composio_gone",
            "The Composio API version this app uses was retired. Update Forma.",
        ),
        429 => AppError::new(
            "composio_rate_limit",
            "The Composio connector service is rate-limiting requests. Try again later.",
        ),
        _ => AppError::new(
            "composio_http",
            "The Composio connector service returned an error. Try again.",
        ),
    }
}
impl Transport for ComposioHttp {
    fn request(
        &self,
        method: reqwest::Method,
        url: reqwest::Url,
        body: Option<String>,
    ) -> Pin<Box<dyn Future<Output = AppResult<Zeroizing<Vec<u8>>>> + Send + '_>> {
        Box::pin(async move {
            // Direct mode: docs-canonical x-api-key only. Sending x-api-key and
            // Bearer together is rejected upstream (401), so the Bearer form is
            // reserved for the relay token below.
            let mut builder = if let (Some(base), Some(token)) =
                (&self.relay_base, &self.relay_token)
            {
                let target = relay_url(base, &url)?;
                let mut header = reqwest::header::HeaderValue::from_str(&format!(
                    "Bearer {}",
                    token.as_str()
                ))
                .map_err(|_| AppError::invalid())?;
                header.set_sensitive(true);
                self.client
                    .request(method, target)
                    .header(reqwest::header::AUTHORIZATION, header)
            } else {
                let (_, api_key) = self.key_header()?;
                self.client
                    .request(method, url)
                    .header("x-api-key", api_key)
            };
            builder = builder.header(reqwest::header::ACCEPT, "application/json");
            if let Some(body) = &body {
                builder = builder
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body.clone());
            }
            let mut response = builder.send().await.map_err(|_| network_error())?;
            if !response.status().is_success() {
                return Err(map_status(response.status()));
            }
            if response
                .content_length()
                .is_some_and(|length| length > MAX_BODY_BYTES as u64)
            {
                return Err(data_error());
            }
            let mut bytes = Zeroizing::new(Vec::new());
            while let Some(chunk) = response.chunk().await.map_err(|_| network_error())? {
                if bytes.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
                    return Err(data_error());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        })
    }
}

fn relay_url(base: &reqwest::Url, url: &reqwest::Url) -> AppResult<reqwest::Url> {
    let path = url.path();
    if !path.starts_with("/api/v3/") || path.len() > 512 {
        return Err(AppError::invalid());
    }
    let mut target = base.clone();
    target.set_path(path);
    target.set_query(url.query());
    Ok(target)
}
fn api(path: &str) -> AppResult<reqwest::Url> {
    reqwest::Url::parse(&format!("{BASE_URL}{path}")).map_err(|_| AppError::invalid())
}
fn decode_json(bytes: &[u8]) -> AppResult<Value> {
    if bytes.is_empty() || bytes.len() > MAX_BODY_BYTES {
        return Err(data_error());
    }
    serde_json::from_slice(bytes).map_err(|_| data_error())
}

fn category_label(value: &Value) -> Option<String> {
    match value {
        Value::String(name) => sanitize_detail(Some(name), 80),
        Value::Object(_) => bounded_str(value, "name", 80)
            .or_else(|| bounded_str(value, "id", 80))
            .map(str::to_owned),
        _ => None,
    }
}
fn parse_service(item: &Value) -> AppResult<ToolkitSummary> {
    let slug = bounded_str(item, "slug", 64)
        .filter(|slug| valid_service_slug(slug))
        .ok_or_else(data_error)?;
    let name = bounded_str(item, "name", 200).ok_or_else(data_error)?;
    let meta = field(item, "meta").unwrap_or(&Value::Null);
    Ok(ToolkitSummary {
        slug: slug.to_owned(),
        name: name.to_owned(),
        description: sanitize_detail(bounded_str(meta, "description", 2_000), 2_000),
        logo: bounded_str(meta, "logo", 2_048)
            .filter(|url| safe_https_url(url))
            .map(str::to_owned),
        categories: field(meta, "categories")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(category_label).take(16).collect())
            .unwrap_or_default(),
        tools_count: field(meta, "tools_count")
            .and_then(Value::as_u64)
            .filter(|count| *count <= 100_000),
        no_auth: field(item, "no_auth")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}
fn parse_services(items: &[Value]) -> Vec<ToolkitSummary> {
    items
        .iter()
        .filter_map(|item| parse_service(item).ok())
        .collect()
}
fn parse_page(value: &Value) -> AppResult<(Vec<Value>, u64)> {
    let items = field(value, "items")
        .and_then(Value::as_array)
        .filter(|items| items.len() <= MAX_PAGE_ITEMS)
        .ok_or_else(data_error)?;
    let total_items = field(value, "total_items")
        .and_then(Value::as_u64)
        .filter(|total| *total <= 100_000_000)
        .ok_or_else(data_error)?;
    Ok((items.clone(), total_items))
}
fn parse_account(item: &Value) -> AppResult<ServiceStatus> {
    let id = bounded_str(item, "id", 64)
        .filter(|id| valid_composio_id(id))
        .ok_or_else(data_error)?;
    let service = field(item, "toolkit")
        .and_then(|toolkit| bounded_str(toolkit, "slug", 64))
        .filter(|slug| valid_service_slug(slug))
        .ok_or_else(data_error)?;
    let raw_status = bounded_str(item, "status", 32).ok_or_else(data_error)?;
    let status = match raw_status {
        "ACTIVE" => "connected",
        "INITIALIZING" | "INITIATED" => "connecting",
        _ => "needs_attention",
    };
    let status_reason = sanitize_detail(bounded_str(item, "status_reason", 500), 200);
    Ok(ServiceStatus {
        id: id.to_owned(),
        service: service.to_owned(),
        status,
        status_detail: status_reason.or(Some(raw_status.to_owned())),
        alias: sanitize_detail(bounded_str(item, "alias", 200), 200),
        word_id: sanitize_detail(bounded_str(item, "word_id", 200), 200),
        connected_at: sanitize_detail(bounded_str(item, "created_at", 64), 64),
    })
}

async fn catalog_page(
    transport: &dyn Transport,
    search: Option<&str>,
    category: Option<&str>,
    limit: u16,
) -> AppResult<(Vec<Value>, u64)> {
    let mut url = api("/toolkits")?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("limit", &limit.to_string());
        if let Some(search) = search {
            pairs.append_pair("search", search);
        }
        if let Some(category) = category {
            pairs.append_pair("category", category);
        }
    }
    let bytes = transport.request(reqwest::Method::GET, url, None).await?;
    parse_page(&decode_json(&bytes)?)
}
async fn categories(transport: &dyn Transport) -> AppResult<Vec<Category>> {
    let url = api("/toolkits/categories")?;
    let bytes = transport.request(reqwest::Method::GET, url, None).await?;
    let (items, _) = parse_page(&decode_json(&bytes)?)?;
    Ok(items
        .iter()
        .take(64)
        .filter_map(|item| {
            let name = bounded_str(item, "name", 80)?;
            let id = bounded_str(item, "id", 64).filter(|id| valid_service_slug(id))?;
            Some(Category {
                name: name.to_owned(),
                id: id.to_owned(),
            })
        })
        .collect())
}
#[derive(Debug)]
struct AuthConfigRow {
    id: String,
    auth_scheme: String,
}
async fn managed_auth_config(transport: &dyn Transport, service: &str) -> AppResult<AuthConfigRow> {
    let mut url = api("/auth_configs")?;
    url.query_pairs_mut()
        .append_pair("toolkit_slug", service)
        .append_pair("is_composio_managed", "true")
        .append_pair("limit", "10");
    let bytes = transport.request(reqwest::Method::GET, url, None).await?;
    let (items, _) = parse_page(&decode_json(&bytes)?)?;
    items
        .iter()
        .filter_map(|item| {
            let id = bounded_str(item, "id", 64).filter(|id| valid_composio_id(id))?;
            let managed = field(item, "is_composio_managed").and_then(Value::as_bool)?;
            let auth_scheme = bounded_str(item, "auth_scheme", 32)?.to_owned();
            managed.then(|| AuthConfigRow {
                id: id.to_owned(),
                auth_scheme,
            })
        })
        // Prefer an OAuth flow; API-key schemes also complete through the link page.
        .min_by_key(|config| {
            if config.auth_scheme.starts_with("OAUTH") {
                0
            } else {
                1
            }
        })
        .ok_or_else(|| {
            AppError::new(
                "composio_unavailable",
                "This service is not available for Composio-managed sign-in yet.",
            )
        })
}
struct LinkSession {
    redirect_url: String,
    connected_account_id: String,
}
async fn initiate_link(
    transport: &dyn Transport,
    auth_config_id: &str,
    callback_url: &str,
) -> AppResult<LinkSession> {
    let url = api("/connected_accounts/link")?;
    let body = json!({
        "auth_config_id": auth_config_id,
        "user_id": LOCAL_USER_ID,
        "callback_url": callback_url,
    })
    .to_string();
    let bytes = transport
        .request(reqwest::Method::POST, url, Some(body))
        .await?;
    let value = decode_json(&bytes)?;
    let redirect_url = bounded_str(&value, "redirect_url", 2_048)
        .filter(|url| composio_host_url(url))
        .ok_or_else(data_error)?;
    let connected_account_id = bounded_str(&value, "connected_account_id", 64)
        .filter(|id| valid_composio_id(id))
        .ok_or_else(data_error)?;
    Ok(LinkSession {
        redirect_url: redirect_url.to_owned(),
        connected_account_id: connected_account_id.to_owned(),
    })
}
async fn connected_accounts(
    transport: &dyn Transport,
    service: Option<&str>,
) -> AppResult<Vec<ServiceStatus>> {
    let mut url = api("/connected_accounts")?;
    url.query_pairs_mut()
        .append_pair("limit", &MAX_LIST_ITEMS.to_string());
    if let Some(service) = service {
        url.query_pairs_mut().append_pair("toolkit_slugs", service);
    }
    let bytes = transport.request(reqwest::Method::GET, url, None).await?;
    let (items, _) = parse_page(&decode_json(&bytes)?)?;
    if items.len() > MAX_ACCOUNTS {
        return Err(data_error());
    }
    Ok(items
        .iter()
        .filter_map(|item| parse_account(item).ok())
        .collect())
}
async fn account(transport: &dyn Transport, id: &str) -> AppResult<ServiceStatus> {
    if !valid_composio_id(id) {
        return Err(AppError::invalid());
    }
    let url = api(&format!("/connected_accounts/{id}"))?;
    let bytes = transport.request(reqwest::Method::GET, url, None).await?;
    parse_account(&decode_json(&bytes)?)
}
async fn disconnect_account(transport: &dyn Transport, id: &str) -> AppResult<()> {
    if !valid_composio_id(id) {
        return Err(AppError::invalid());
    }
    let url = api(&format!("/connected_accounts/{id}"))?;
    let bytes = transport
        .request(reqwest::Method::DELETE, url, None)
        .await?;
    // A missing account is already disconnected; retryable cleanup stays honest.
    let value: Value = if bytes.is_empty() {
        Value::Null
    } else {
        decode_json(&bytes)?
    };
    if let Some(deleted) = value.get("id").and_then(Value::as_str) {
        if deleted != id {
            return Err(data_error());
        }
    }
    Ok(())
}
/// Polls the authoritative account status until it settles. The loopback
/// arrival is only an accelerator; API status is the sole source of truth.
async fn wait_for_active(
    transport: &dyn Transport,
    id: &str,
    deadline: Instant,
    cancel: &CancellationToken,
) -> AppResult<()> {
    loop {
        if cancel.is_cancelled() {
            return Err(stale_error());
        }
        let status = account(transport, id).await?;
        match status.status {
            "connected" => return Ok(()),
            "connecting" => {}
            _ => return Err(AppError::new(
                "composio_denied",
                "The service connection did not complete. You can retry from the Services page.",
            )),
        }
        if Instant::now() >= deadline {
            return Err(timeout_error());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

// A dropped/cancelled command future also retires its own attempt and closes
// the bound loopback socket; stale cleanup cannot cancel a replacement.
struct LaunchLease<'a> {
    core: &'a ComposioCore,
    id: &'a str,
    admitted: bool,
}
impl Drop for LaunchLease<'_> {
    fn drop(&mut self) {
        if self.admitted {
            self.core
                .finish_attempt(self.id, AttemptPhase::Failed, Some(browser_error()), None);
        }
    }
}

#[tauri::command]
pub async fn composio_status(window: WebviewWindow, app: AppHandle) -> AppResult<Snapshot> {
    crate::first_party(&window)?;
    let core = core(&app)?;
    let attempt = core.snapshot_attempt();
    let revision = core.revision();
    let relay = core.relay.is_some();
    let key_configured = relay || core.keys.has();
    // While the native callback thread drives a live attempt, snapshots stay
    // in memory: no extra Composio round trips from renderer polling (F9).
    // That thread emits composio:changed on every transition; the snapshot
    // after it reconciles the authoritative account list.
    let attempt_live = attempt.as_ref().is_some_and(|status| {
        matches!(
            status.phase,
            AttemptPhase::Pending | AttemptPhase::Confirming
        )
    });
    let items = if key_configured && !attempt_live {
        let http = core.http()?;
        connected_accounts(&http, None).await?
    } else {
        Vec::new()
    };
    Ok(Snapshot {
        revision,
        key_configured,
        relay,
        attempt,
        items,
    })
}
#[tauri::command]
pub async fn composio_catalog(
    window: WebviewWindow,
    app: AppHandle,
    search: Option<String>,
    category: Option<String>,
    limit: Option<u16>,
) -> AppResult<CatalogPage> {
    crate::first_party(&window)?;
    let search = search
        .map(|value| -> AppResult<String> {
            crate::validation::single_line(&value, 120, true)?;
            Ok(value.trim().to_owned())
        })
        .transpose()?
        .filter(|value| !value.is_empty());
    let category = category
        .map(|value| {
            if valid_service_slug(&value) {
                Ok(value)
            } else {
                Err(AppError::invalid())
            }
        })
        .transpose()?;
    let limit = limit.unwrap_or(100).clamp(1, MAX_CATALOG_LIMIT);
    let core = core(&app)?;
    let http = core.http()?;
    let (items, total_items) =
        catalog_page(&http, search.as_deref(), category.as_deref(), limit).await?;
    // A malformed row is skipped, never a whole-page failure (F6); page-level
    // bounds (size, totals) still fail closed.
    Ok(CatalogPage {
        items: parse_services(&items),
        total_items,
    })
}
#[tauri::command]
pub async fn composio_categories(
    window: WebviewWindow,
    app: AppHandle,
) -> AppResult<Vec<Category>> {
    crate::first_party(&window)?;
    let core = core(&app)?;
    let http = core.http()?;
    categories(&http).await
}
#[tauri::command]
pub async fn composio_start(
    window: WebviewWindow,
    app: AppHandle,
    service: String,
) -> AppResult<StartResponse> {
    crate::first_party(&window)?;
    if !valid_service_slug(&service) {
        return Err(AppError::invalid());
    }
    if !super::opener::supported() {
        return Err(AppError::new(
            "composio_disabled",
            "Service sign-in through the system browser is not supported on this platform yet.",
        ));
    }
    let core = core(&app)?;
    let http = core.http()?;
    let loopback = Loopback::bind()?;
    let callback_url = loopback.redirect_uri(CALLBACK_PATH);
    let (id, cancel) = core.begin_attempt(&service, CALLBACK_TIMEOUT)?;
    // Any failure before the browser opens retires the attempt immediately.
    let configured = match managed_auth_config(&http, &service).await {
        Ok(config) => config,
        Err(error) => {
            core.finish_attempt(&id, AttemptPhase::Failed, Some(error.clone()), None);
            emit_change(&app);
            return Err(error);
        }
    };
    let link = match initiate_link(&http, &configured.id, &callback_url).await {
        Ok(link) => link,
        Err(error) => {
            core.finish_attempt(&id, AttemptPhase::Failed, Some(error.clone()), None);
            emit_change(&app);
            return Err(error);
        }
    };
    core.attach_account(&id, &link.connected_account_id);
    emit_change(&app);
    // The redirect URL stays native-owned and is only passed to the system
    // browser opener; the renderer never receives it.
    let launched = {
        let mut lease = LaunchLease {
            core: &core,
            id: &id,
            admitted: false,
        };
        let launch = {
            core.check_attempt(&id)?;
            lease.admitted = true;
            super::opener::spawn(&link.redirect_url)?
        };
        let launched = tokio::select! {
            biased;
            _ = cancel.cancelled() => false,
            _ = tokio::time::sleep(OPEN_TIMEOUT) => false,
            result = launch => result.is_ok(),
        };
        core.check_attempt(&id)?;
        lease.admitted = false;
        launched
    };
    if !launched {
        let error = browser_error();
        core.finish_attempt(&id, AttemptPhase::Failed, Some(error.clone()), None);
        emit_change(&app);
        return Err(error);
    }
    let account_id = link.connected_account_id.clone();
    let service_thread = service.clone();
    let deadline = Instant::now() + CALLBACK_TIMEOUT;
    let thread_core = Arc::clone(&core);
    let thread_http = ComposioHttp {
        client: http.client.clone(),
        key: Arc::clone(&http.key),
        relay_base: None,
        relay_token: None,
    };
    let app_handle = app.clone();
    let thread_id = id.clone();
    let spawn = std::thread::Builder::new()
        .name("composio-callback".into())
        .spawn(move || {
            let arrival = loopback.wait_for_arrival(CALLBACK_PATH, deadline, &cancel);
            if arrival.is_ok() && thread_core.mark_confirming(&thread_id) {
                emit_change(&app_handle);
            }
            tauri::async_runtime::spawn(async move {
                let result = wait_for_active(&thread_http, &account_id, deadline, &cancel).await;
                let (phase, error) = match result {
                    Ok(()) => (AttemptPhase::Connected, None),
                    Err(error) if error.code == "composio_stale" => (AttemptPhase::Cancelled, None),
                    Err(error) => (AttemptPhase::Failed, Some(error)),
                };
                if phase == AttemptPhase::Connected {
                    thread_core.resolve_prompt_for_service(&service_thread);
                }
                thread_core.finish_attempt(&thread_id, phase, error, None);
                emit_change(&app_handle);
            });
        });
    match spawn {
        Ok(_) => {
            emit_change(&app);
            Ok(StartResponse { attempt_id: id })
        }
        Err(_) => {
            let error = AppError::new(
                "connector_loopback",
                "The temporary sign-in callback listener is unavailable.",
            );
            core.finish_attempt(&id, AttemptPhase::Failed, Some(error.clone()), None);
            emit_change(&app);
            Err(error)
        }
    }
}
#[tauri::command]
pub async fn composio_cancel(
    window: WebviewWindow,
    app: AppHandle,
    attempt_id: String,
) -> AppResult<()> {
    crate::first_party(&window)?;
    let parsed = uuid::Uuid::parse_str(&attempt_id).map_err(|_| AppError::invalid())?;
    if parsed.is_nil() || parsed.to_string() != attempt_id {
        return Err(AppError::invalid());
    }
    let core = core(&app)?;
    core.cancel_attempt(&attempt_id)?;
    emit_change(&app);
    Ok(())
}
#[tauri::command]
pub async fn composio_disconnect(
    window: WebviewWindow,
    app: AppHandle,
    connected_account_id: String,
) -> AppResult<()> {
    crate::first_party(&window)?;
    if !valid_composio_id(&connected_account_id) {
        return Err(AppError::invalid());
    }
    let core = core(&app)?;
    let http = core.http()?;
    let result = disconnect_account(&http, &connected_account_id).await;
    if result.is_ok() {
        // A disconnect changes connection state; stale prompt suppression lifts.
        core.resolve_all_prompts();
    }
    emit_change(&app);
    result
}
#[tauri::command]
pub async fn composio_prompt_dismiss(
    window: WebviewWindow,
    app: AppHandle,
    workspace_id: String,
    service: String,
) -> AppResult<()> {
    crate::first_party(&window)?;
    uuid::Uuid::parse_str(&workspace_id).map_err(|_| AppError::invalid())?;
    if !valid_service_slug(&service) {
        return Err(AppError::invalid());
    }
    let core = core(&app)?;
    core.dismiss_prompt(&workspace_id, &service);
    Ok(())
}
#[tauri::command]
pub async fn composio_key_save(
    window: WebviewWindow,
    app: AppHandle,
    api_key: String,
) -> AppResult<()> {
    crate::first_party(&window)?;
    validate_api_key(&api_key)?;
    let core = core(&app)?;
    core.keys.write(&api_key)?;
    emit_change(&app);
    Ok(())
}
#[tauri::command]
pub async fn composio_key_remove(window: WebviewWindow, app: AppHandle) -> AppResult<()> {
    crate::first_party(&window)?;
    let core = core(&app)?;
    core.keys.remove()?;
    emit_change(&app);
    Ok(())
}
fn validate_api_key(value: &str) -> AppResult<()> {
    if !value.starts_with("ak_")
        || value.len() < 16
        || value.len() > 4096
        || !value.bytes().all(|byte| (33..=126).contains(&byte))
    {
        return Err(AppError::new(
            "invalid_input",
            "That does not look like a Composio API key. Composio keys start with ak_.",
        ));
    }
    Ok(())
}

/// Hermes runtime bridge. Python has no Composio access; the host returns only
/// bounded, host-validated statuses and never the API key or redirect URLs.
pub async fn dispatch_runtime_tool(
    app: &AppHandle,
    workspace_id: &str,
    run_id: &str,
    tool_name: &str,
    args: &Value,
) -> AppResult<Value> {
    let core = core(app)?;
    match tool_name {
        RUNTIME_LIST_TOOL => list_connected_services(&core).await,
        RUNTIME_CONNECT_TOOL => {
            request_service_connection(app, &core, workspace_id, run_id, args).await
        }
        _ => Err(AppError::invalid()),
    }
}
async fn list_connected_services(core: &Arc<ComposioCore>) -> AppResult<Value> {
    if !core.keys.has() {
        return Ok(list_value(&[], false));
    }
    let http = core.http()?;
    let accounts = connected_accounts(&http, None).await?;
    Ok(list_value(&accounts, true))
}
fn list_value(accounts: &[ServiceStatus], configured: bool) -> Value {
    let items: Vec<Value> = accounts
        .iter()
        .take(MAX_LIST_ITEMS)
        .map(|account| json!({ "slug": account.service, "status": account.status }))
        .collect();
    json!({
        "kind": "connectedServices",
        "configured": configured,
        "items": items,
    })
}
async fn request_service_connection(
    app: &AppHandle,
    core: &Arc<ComposioCore>,
    workspace_id: &str,
    run_id: &str,
    args: &Value,
) -> AppResult<Value> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        service: String,
    }
    let args: Args = serde_json::from_value(args.clone()).map_err(|_| AppError::invalid())?;
    if !valid_service_slug(&args.service) {
        return Err(AppError::invalid());
    }
    let (result, emit_prompt) =
        evaluate_connection(core, workspace_id, run_id, &args.service).await?;
    // The prompt is host-rendered from a structured event, never message text,
    // and stays deduplicated per workspace until resolved or dismissed.
    if emit_prompt {
        let _ = app.emit_to(
            tauri::EventTarget::webview_window("main"),
            PROMPT_EVENT,
            json!({
                "workspaceId": workspace_id,
                "runId": run_id,
                "service": args.service,
            }),
        );
    }
    Ok(result)
}
async fn evaluate_connection(
    core: &Arc<ComposioCore>,
    workspace_id: &str,
    run_id: &str,
    service: &str,
) -> AppResult<(Value, bool)> {
    let status = |name: &str| {
        json!({
            "kind": "serviceConnectionRequest",
            "service": service,
            "status": name,
        })
    };
    if !core.keys.has() {
        return Ok((
            json!({
                "kind": "serviceConnectionRequest",
                "service": service,
                "status": "failed",
                "detail": "Composio is not configured on this device yet.",
            }),
            false,
        ));
    }
    let http = core.http()?;
    let accounts = connected_accounts(&http, Some(service)).await?;
    if accounts.iter().any(|account| account.status == "connected") {
        return Ok((status("already_connected"), false));
    }
    if let Some(attempt) = core.live_attempt() {
        if attempt.service == service {
            return Ok((status("initiated"), false));
        }
    }
    Ok((
        status("pending_user_action"),
        core.should_prompt(workspace_id, service, run_id),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::keychain::tests::MemoryKeys;
    use std::collections::VecDeque;

    struct FixtureTransport {
        requests: Mutex<Vec<(String, String, Option<String>)>>,
        responses: Mutex<VecDeque<AppResult<String>>>,
    }
    impl FixtureTransport {
        fn new(responses: Vec<AppResult<String>>) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into()),
            }
        }
        fn recorded(&self) -> Vec<(String, String, Option<String>)> {
            self.requests.lock().unwrap().clone()
        }
    }
    impl Transport for FixtureTransport {
        fn request(
            &self,
            method: reqwest::Method,
            url: reqwest::Url,
            body: Option<String>,
        ) -> Pin<Box<dyn Future<Output = AppResult<Zeroizing<Vec<u8>>>> + Send + '_>> {
            let entry = (method.to_string(), url.as_str().to_owned(), body);
            self.requests.lock().unwrap().push(entry);
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(data_error()));
            Box::pin(async move { response.map(|text| Zeroizing::new(text.into_bytes())) })
        }
    }
    fn ok(body: serde_json::Value) -> AppResult<String> {
        Ok(body.to_string())
    }
    fn core_with(keys: MemoryKeys) -> Arc<ComposioCore> {
        Arc::new(ComposioCore {
            inner: Mutex::new(Inner {
                revision: 0,
                pending: None,
                attempt_status: None,
                prompts: Vec::new(),
            }),
            keys: Arc::new(keys),
            client: build_client().unwrap(),
            relay: None,
        })
    }
    fn service_fixture() -> serde_json::Value {
        json!({
            "slug": "github",
            "name": "GitHub",
            "type": "native",
            "no_auth": false,
            "meta": {
                "description": "Code hosting",
                "logo": "https://cdn.example.com/github.png",
                "app_url": "https://github.com",
                "categories": [{"name": "Developer Tools", "id": "developer-tools"}, "popular"],
                "tools_count": 42,
                "version": "1"
            }
        })
    }
    fn page(items: serde_json::Value, total: u64) -> serde_json::Value {
        json!({
            "items": items,
            "next_cursor": null,
            "total_pages": 1,
            "current_page": 1,
            "total_items": total,
        })
    }

    #[tokio::test]
    async fn catalog_sends_bounded_query_and_parses_snake_and_camel() {
        let transport = FixtureTransport::new(vec![
            ok(page(json!([service_fixture()]), 1_512)),
            ok(page(
                json!([{
                    "slug": "notion", "name": "Notion",
                    "meta": {"description": "Docs", "toolsCount": 7, "categories": ["productivity"]}
                }]),
                2,
            )),
        ]);
        let (items, total) = catalog_page(&transport, Some("git"), Some("developer-tools"), 100)
            .await
            .unwrap();
        assert_eq!(total, 1_512);
        let parsed = parse_service(&items[0]).unwrap();
        assert_eq!(parsed.slug, "github");
        assert_eq!(parsed.name, "GitHub");
        assert_eq!(parsed.categories, vec!["Developer Tools", "popular"]);
        assert_eq!(parsed.tools_count, Some(42));
        assert!(parsed
            .logo
            .as_deref()
            .is_some_and(|logo| logo.starts_with("https://")));
        let recorded = transport.recorded();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0]
            .1
            .starts_with("https://backend.composio.dev/api/v3/toolkits?"));
        assert!(recorded[0].1.contains("limit=100"));
        assert!(recorded[0].1.contains("search=git"));
        assert!(recorded[0].1.contains("category=developer-tools"));
        // camelCase field tolerance
        let (items, _) = catalog_page(&transport, None, None, 30).await.unwrap();
        let parsed = parse_service(&items[0]).unwrap();
        assert_eq!(parsed.tools_count, Some(7));
        assert_eq!(parsed.categories, vec!["productivity"]);
    }
    #[tokio::test]
    async fn catalog_rejects_oversized_pages_and_unsafe_logos() {
        let too_many: Vec<_> = (0..=MAX_PAGE_ITEMS)
            .map(|index| json!({"slug": format!("s{index}"), "name": "S"}))
            .collect();
        for body in [
            ok(page(json!(too_many), 10)),
            ok(json!({"items": []})),
            ok(json!({"items": []})),
        ] {
            let transport = FixtureTransport::new(vec![body]);
            assert!(catalog_page(&transport, None, None, 100).await.is_err());
        }
        let transport = FixtureTransport::new(vec![ok(page(
            json!([{
                "slug": "evil", "name": "Evil",
                "meta": {"logo": "http://cdn.example.com/x.png"}
            }]),
            1,
        ))]);
        let (items, _) = catalog_page(&transport, None, None, 10).await.unwrap();
        assert_eq!(parse_service(&items[0]).unwrap().logo, None);
        let transport = FixtureTransport::new(vec![ok(page(json!([{"name": "NoSlug"}]), 1))]);
        let (items, _) = catalog_page(&transport, None, None, 10).await.unwrap();
        assert!(parse_service(&items[0]).is_err());
    }
    #[tokio::test]
    async fn account_statuses_map_to_honest_vocabulary() {
        let items = json!([
            {"id": "aaaaaaaaaa", "toolkit": {"slug": "github"}, "status": "ACTIVE",
             "alias": "work", "word_id": "github_red-castle", "created_at": "2026-09-12T00:00:00Z"},
            {"id": "bbbbbbbbbb", "toolkit": {"slug": "notion"}, "status": "INITIATED",
             "status_reason": "waiting for user"},
            {"id": "cccccccccc", "toolkit": {"slug": "linear"}, "status": "FAILED",
             "status_reason": "oauth_denied"}
        ]);
        let transport = FixtureTransport::new(vec![ok(page(items, 3))]);
        let accounts = connected_accounts(&transport, None).await.unwrap();
        assert_eq!(accounts[0].status, "connected");
        assert_eq!(accounts[0].alias.as_deref(), Some("work"));
        assert_eq!(accounts[0].word_id.as_deref(), Some("github_red-castle"));
        assert_eq!(accounts[1].status, "connecting");
        assert_eq!(
            accounts[1].status_detail.as_deref(),
            Some("waiting for user")
        );
        assert_eq!(accounts[2].status, "needs_attention");
        assert_eq!(accounts[2].status_detail.as_deref(), Some("oauth_denied"));
        assert!(transport.recorded()[0].1.contains("/connected_accounts"));
    }
    #[tokio::test]
    async fn managed_auth_config_prefers_oauth_and_requires_managed() {
        let items = json!([
            {"id": "dddddddddd", "auth_scheme": "API_KEY", "is_composio_managed": true},
            {"id": "eeeeeeeeee", "auth_scheme": "OAUTH2", "is_composio_managed": true},
            {"id": "ffffffffff", "auth_scheme": "OAUTH2", "is_composio_managed": false}
        ]);
        let transport = FixtureTransport::new(vec![ok(page(items, 3))]);
        let config = managed_auth_config(&transport, "github").await.unwrap();
        assert_eq!(config.id, "eeeeeeeeee");
        assert!(transport.recorded()[0]
            .1
            .contains("toolkit_slug=github&is_composio_managed=true"));
        let transport = FixtureTransport::new(vec![ok(page(
            json!([{"id": "ffffffffff", "auth_scheme": "OAUTH2", "is_composio_managed": false}]),
            1,
        ))]);
        assert_eq!(
            managed_auth_config(&transport, "github")
                .await
                .unwrap_err()
                .code,
            "composio_unavailable"
        );
    }
    #[tokio::test]
    async fn link_initiation_body_is_exact_and_redirect_url_is_composio_only() {
        let transport = FixtureTransport::new(vec![ok(json!({
            "link_token": "synthetic-link-token",
            "redirect_url": "https://platform.composio.dev/connect/abc",
            "expires_at": "2026-09-12T01:00:00Z",
            "connected_account_id": "abc123def456"
        }))]);
        let link = initiate_link(
            &transport,
            "eeeeeeeeee",
            "http://127.0.0.1:45678/composio/callback",
        )
        .await
        .unwrap();
        assert_eq!(link.connected_account_id, "abc123def456");
        let recorded = transport.recorded();
        assert!(recorded[0].1.ends_with("/connected_accounts/link"));
        let body: serde_json::Value =
            serde_json::from_str(recorded[0].2.as_deref().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({
                "auth_config_id": "eeeeeeeeee",
                "user_id": "forma-desktop",
                "callback_url": "http://127.0.0.1:45678/composio/callback"
            })
        );
        for redirect in [
            "http://platform.composio.dev/connect/abc",
            "https://evil.example.com/connect",
            "https://user:pass@platform.composio.dev/x",
            "https://platform.composio.dev/x#fragment",
            "https://composio.dev.evil.example/x",
        ] {
            let transport = FixtureTransport::new(vec![ok(json!({
                "redirect_url": redirect, "connected_account_id": "abc123def456"
            }))]);
            assert!(
                initiate_link(&transport, "eeeeeeeeee", "http://127.0.0.1:1/x")
                    .await
                    .is_err(),
                "redirect {redirect}"
            );
        }
    }
    #[tokio::test]
    async fn disconnect_is_idempotent_on_missing_and_verifies_identity() {
        for (body, expect_ok) in [
            (r#"{"id":"abc123def456"}"#, true),
            ("", true),
            (r#"{"id":"other12345"}"#, false),
        ] {
            let transport = FixtureTransport::new(vec![Ok(body.to_owned())]);
            let result = disconnect_account(&transport, "abc123def456").await;
            assert_eq!(result.is_ok(), expect_ok, "body {body}");
            assert!(transport.recorded()[0]
                .1
                .ends_with("/connected_accounts/abc123def456"));
        }
        assert!(
            disconnect_account(&FixtureTransport::new(vec![]), "../escape")
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn wait_for_active_confirms_settled_states_without_faking() {
        let settled: Vec<(&str, &str)> = vec![
            ("ACTIVE", "ok"),
            ("FAILED", "composio_denied"),
            ("EXPIRED", "composio_denied"),
        ];
        for (status, code) in settled {
            let transport = FixtureTransport::new(vec![ok(json!({
                "id": "abc123def456", "toolkit": {"slug": "github"}, "status": status
            }))]);
            let cancel = CancellationToken::new();
            let result = wait_for_active(
                &transport,
                "abc123def456",
                Instant::now() + Duration::from_secs(5),
                &cancel,
            )
            .await;
            assert_eq!(result.err().map(|error| error.code).unwrap_or("ok"), code);
        }
        let cancel = CancellationToken::new();
        cancel.cancel();
        let transport = FixtureTransport::new(vec![ok(json!({
            "id": "abc123def456", "toolkit": {"slug": "github"}, "status": "INITIATED"
        }))]);
        assert_eq!(
            wait_for_active(
                &transport,
                "abc123def456",
                Instant::now() + Duration::from_secs(5),
                &cancel
            )
            .await
            .unwrap_err()
            .code,
            "composio_stale"
        );
    }
    #[test]
    fn attempt_is_single_flight_bounded_and_honest() {
        let core = core_with(MemoryKeys::default());
        let (first, cancel) = core
            .begin_attempt("github", Duration::from_secs(60))
            .unwrap();
        assert!(core
            .begin_attempt("notion", Duration::from_secs(60))
            .is_err());
        assert_eq!(
            core.snapshot_attempt().unwrap().phase,
            AttemptPhase::Pending
        );
        assert!(core.attach_account(&first, "abc123def456"));
        assert_eq!(
            core.snapshot_attempt()
                .unwrap()
                .connected_account_id
                .as_deref(),
            Some("abc123def456")
        );
        assert!(core.mark_confirming(&first));
        assert_eq!(
            core.snapshot_attempt().unwrap().phase,
            AttemptPhase::Confirming
        );
        // A cancelled attempt retires and frees the single-flight slot.
        core.cancel_attempt(&first).unwrap();
        assert!(cancel.is_cancelled());
        assert_eq!(
            core.snapshot_attempt().unwrap().phase,
            AttemptPhase::Cancelled
        );
        // A terminal attempt frees the single-flight slot (F1): a second cancel
        // is not_found, a finish is a no-op, and a new attempt can begin.
        assert_eq!(core.cancel_attempt(&first).unwrap_err().code, "not_found");
        assert!(!core.finish_attempt(&first, AttemptPhase::Connected, None, None));
        assert!(core.live_attempt().is_none());
        assert!(core.snapshot_attempt().is_some());
        let (second, _) = core
            .begin_attempt("notion", Duration::from_secs(60))
            .unwrap();
        assert_ne!(first, second);
        core.finish_attempt(&second, AttemptPhase::Failed, Some(network_error()), None);
        let snapshot = core.snapshot_attempt().unwrap();
        assert_eq!(snapshot.phase, AttemptPhase::Failed);
        assert_eq!(snapshot.error.as_ref().unwrap().code, "composio_network");
        let (fourth, _) = core
            .begin_attempt("figma", Duration::from_secs(60))
            .unwrap();
        core.finish_attempt(&fourth, AttemptPhase::Connected, None, None);
        // Deadline expiry retires stale attempts with an honest timeout and
        // also frees the slot for the next attempt.
        let (third, _) = core.begin_attempt("slack", Duration::ZERO).unwrap();
        let snapshot = core.snapshot_attempt().unwrap();
        assert_eq!(snapshot.id, third);
        assert_eq!(snapshot.phase, AttemptPhase::Failed);
        assert_eq!(snapshot.error.as_ref().unwrap().code, "composio_timeout");
        assert!(core.begin_attempt("asana", Duration::from_secs(60)).is_ok());
    }

    #[test]
    fn prompt_suppression_lifts_on_dismissal_resolve_and_ttl() {
        let core = core_with(MemoryKeys::default());
        assert!(core.should_prompt("w1", "github", "r1"));
        assert!(
            !core.should_prompt("w1", "github", "r2"),
            "suppressed while live"
        );
        assert!(
            core.should_prompt("w2", "github", "r3"),
            "other workspace unaffected"
        );
        // Dismissal lifts suppression for that workspace+service (F3).
        core.dismiss_prompt("w1", "github");
        assert!(core.should_prompt("w1", "github", "r4"));
        // A connection resolves suppression for the service everywhere.
        assert!(!core.should_prompt("w1", "github", "r5"));
        core.resolve_prompt_for_service("github");
        assert!(core.should_prompt("w1", "github", "r6"));
        assert!(core.should_prompt("w2", "github", "r7"));
        core.resolve_all_prompts();
        assert!(core.should_prompt("w2", "github", "r8"));
        // TTL expiry re-arms re-emission instead of suppressing forever (F3).
        assert!(!core.should_prompt("w2", "github", "r9"));
        {
            let mut inner = core.inner.lock().unwrap();
            for record in &mut inner.prompts {
                record.at = record
                    .at
                    .checked_sub(PROMPT_TTL + Duration::from_secs(1))
                    .unwrap();
            }
        }
        assert!(
            core.should_prompt("w2", "github", "r10"),
            "expired record re-emits"
        );
    }

    #[tokio::test]
    async fn malformed_rows_are_skipped_not_page_fatal() {
        let mixed = json!([
            service_fixture(),
            {"name": "MissingSlug"},
            {"slug": "broken-name"},
            {"slug": "linear", "name": "Linear"}
        ]);
        let parsed = parse_services(mixed.as_array().unwrap());
        assert_eq!(
            parsed
                .iter()
                .map(|service| service.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["github", "linear"]
        );
        let accounts = json!([
            {"id": "abc123def456", "toolkit": {"slug": "github"}, "status": "ACTIVE"},
            {"id": "nodomain-nope", "status": "ACTIVE"},
            {"toolkit": {"slug": "linear"}, "status": "ACTIVE"}
        ]);
        let transport = FixtureTransport::new(vec![ok(page(accounts, 3))]);
        let parsed = connected_accounts(&transport, None).await.unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].service, "github");
        let cats = json!([
            {"name": "Developer Tools", "id": "developer-tools"},
            {"name": "No Id"},
            {"name": "Bad Id", "id": "NOT-VALID"},
            {"name": "Productivity", "id": "productivity"}
        ]);
        let transport = FixtureTransport::new(vec![ok(page(cats, 4))]);
        let parsed = categories(&transport).await.unwrap();
        assert_eq!(
            parsed
                .iter()
                .map(|category| category.id.as_str())
                .collect::<Vec<_>>(),
            vec!["developer-tools", "productivity"]
        );
    }

    #[test]
    fn live_attempt_and_status_mirror_split_after_terminals() {
        let core = core_with(MemoryKeys::default());
        let (id, _) = core
            .begin_attempt("github", Duration::from_secs(60))
            .unwrap();
        assert!(core.live_attempt().is_some());
        core.finish_attempt(&id, AttemptPhase::Connected, None, None);
        assert!(
            core.live_attempt().is_none(),
            "terminal frees the live slot"
        );
        let snapshot = core.snapshot_attempt().unwrap();
        assert_eq!(snapshot.phase, AttemptPhase::Connected);
        assert_eq!(snapshot.id, id);
    }
    #[tokio::test]
    async fn runtime_tools_return_honest_statuses_and_dedupe_prompts() {
        // Unconfigured: list reports configured=false; connect reports failed.
        let unconfigured = core_with(MemoryKeys::default());
        assert_eq!(
            list_connected_services(&unconfigured).await.unwrap(),
            json!({"kind": "connectedServices", "configured": false, "items": []})
        );
        let (result, emit) = evaluate_connection(&unconfigured, "w1", "r1", "github")
            .await
            .unwrap();
        assert_eq!(result["status"], "failed");
        assert_eq!(result["kind"], "serviceConnectionRequest");
        assert!(!emit);
        // Configured with an ACTIVE account: already_connected, no prompt.
        let keys = MemoryKeys::default();
        keys.write("ak_synthetic-test-key-1234").unwrap();
        let core = core_with(keys);
        let transport = FixtureTransport::new(vec![ok(page(
            json!([{"id": "abc123def456", "toolkit": {"slug": "github"}, "status": "ACTIVE"}]),
            1,
        ))]);
        // evaluate_connection uses the real http client; drive the same logic
        // through the transport-backed pieces it composes.
        let accounts = connected_accounts(&transport, Some("github"))
            .await
            .unwrap();
        assert!(accounts.iter().any(|account| account.status == "connected"));
        // Prompt dedupe: first ask emits, repeat is silent until resolved.
        assert!(core.should_prompt("w1", "github", "r1"));
        assert!(!core.should_prompt("w1", "github", "r2"));
        assert!(core.should_prompt("w2", "github", "r3"));
        core.resolve_prompt_for_service("github");
        assert!(core.should_prompt("w1", "github", "r4"));
        // List value projection keeps slugs and statuses only.
        assert_eq!(
            list_value(&accounts, true),
            json!({"kind": "connectedServices", "configured": true,
                   "items": [{"slug": "github", "status": "connected"}]})
        );
    }
    #[test]
    fn key_and_slug_validation_shapes() {
        assert!(valid_service_slug("github"));
        assert!(valid_service_slug("google_calendar"));
        assert!(!valid_service_slug("Google"));
        assert!(!valid_service_slug("-lead"));
        assert!(!valid_service_slug(""));
        assert!(!valid_service_slug(&"a".repeat(65)));
        assert!(valid_composio_id("abc123def456"));
        assert!(!valid_composio_id("short"));
        assert!(!valid_composio_id("has space!!"));
        assert!(validate_api_key("ak_synthetic-key-123456").is_ok());
        for bad in [
            "sk_wrongprefix-key-123456",
            "ak_short",
            "ak_with space",
            "ak_\nheader-injection",
            "",
        ] {
            assert!(validate_api_key(bad).is_err(), "key {bad:?}");
        }
    }
    #[test]
    fn key_header_is_bearer_marked_sensitive_and_never_in_errors() {
        let http = ComposioHttp {
            client: build_client().unwrap(),
            key: Arc::new(Zeroizing::new("ak_synthetic-key-123456".to_owned())),
            relay_base: None,
            relay_token: None,
        };
        let (bearer, api_key) = http.key_header().unwrap();
        assert_eq!(bearer.to_str().unwrap(), "Bearer ak_synthetic-key-123456");
        assert!(bearer.is_sensitive());
        assert!(
            api_key.is_sensitive(),
            "x-api-key must also be marked sensitive"
        );
        assert_eq!(api_key.to_str().unwrap(), "ak_synthetic-key-123456");
        // Error messages are static and never embed the key.
        for error in [
            network_error(),
            data_error(),
            map_status(reqwest::StatusCode::UNAUTHORIZED),
            map_status(reqwest::StatusCode::TOO_MANY_REQUESTS),
        ] {
            assert!(!error.message.contains("ak_"));
        }
    }
    #[test]
    #[test]
    fn relay_url_maps_api_paths_and_rejects_others() {
        let base = reqwest::Url::parse("http://100.90.255.43:9231").unwrap();
        let mapped = relay_url(
            &base,
            &reqwest::Url::parse("https://backend.composio.dev/api/v3/toolkits?search=g&limit=30")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            mapped.as_str(),
            "http://100.90.255.43:9231/api/v3/toolkits?search=g&limit=30"
        );
        let foreign = reqwest::Url::parse("https://backend.composio.dev/v2/toolkits").unwrap();
        assert!(relay_url(&base, &foreign).is_err());
    }

    #[test]
    fn relay_mode_reports_configured_without_a_direct_key() {
        let core = Arc::new(ComposioCore {
            inner: Mutex::new(Inner {
                revision: 0,
                pending: None,
                attempt_status: None,
                prompts: Vec::new(),
            }),
            keys: Arc::new(MemoryKeys::default()),
            client: build_client().unwrap(),
            relay: Some((
                reqwest::Url::parse("http://127.0.0.1:9").unwrap(),
                Arc::new(Zeroizing::new("forma-relay-test-token-0123456789".to_owned())),
            )),
        });
        assert!(!core.keys.has());
        let http = core.http().unwrap();
        assert!(http.relay_base.is_some() && http.relay_token.is_some());
    }

    fn snapshots_never_contain_key_material() {
        let keys = MemoryKeys::default();
        keys.write("ak_synthetic-key-123456").unwrap();
        let core = core_with(keys);
        let (id, _) = core
            .begin_attempt("github", Duration::from_secs(60))
            .unwrap();
        core.attach_account(&id, "abc123def456");
        core.finish_attempt(&id, AttemptPhase::Failed, Some(network_error()), None);
        let snapshot = core.snapshot_attempt().unwrap();
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("ak_"));
        assert!(encoded.contains("github"));
        assert!(encoded.contains("failed"));
    }
}
