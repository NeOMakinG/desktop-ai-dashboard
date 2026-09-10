//! Ephemeral operator grants. This module never confers authority on model arguments.
use super::*;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

pub const MAX_GRANT_SECONDS: i64 = 3600;
pub const MAX_READ_SECONDS: i64 = 120;
const MAX_GRANTS: usize = 128;
const MAX_CANCELLED_RUNS: usize = 256;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum ReadOperation {
    #[serde(rename = "forma_gmail_list_metadata")]
    GmailListMetadata,
    #[serde(rename = "forma_calendar_list_events")]
    CalendarListEvents,
}
impl ReadOperation {
    pub fn category(self) -> &'static str {
        match self {
            Self::GmailListMetadata => "gmail.metadata",
            Self::CalendarListEvents => "calendar.events",
        }
    }
    pub fn scope_allowed(self, scopes: &[String]) -> bool {
        match self {
            Self::GmailListMetadata => scopes
                .iter()
                .any(|s| s == ALLOWED_SCOPES[0] || s == ALLOWED_SCOPES[1]),
            Self::CalendarListEvents => scopes.iter().any(|s| s == ALLOWED_SCOPES[2]),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EgressConsent {
    pub runtime_origin: String,
    pub model_origin: String,
    pub data_categories: Vec<String>,
    pub consented: bool,
    // Acknowledgment is not evidence that a remote service enforces this policy.
    pub retention: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrantRequest {
    pub workspace_id: String,
    pub device_id: String,
    pub connection_id: String,
    pub operations: Vec<ReadOperation>,
    pub expires_at: String,
    pub account_read_consented: bool,
    pub egress: EgressConsent,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorGrant {
    pub grant_id: String,
    pub generation: u64,
    #[serde(flatten)]
    pub scope: GrantRequest,
}

/// Trusted native runtime supplies the envelope from its authenticated run state.
/// Do not expose this dispatcher as a generated-webview command or use model fields
/// as workspace, run, device, origin, expiry or selected-model identity.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeReadContext {
    pub workspace_id: String,
    pub run_id: String,
    pub device_id: String,
    pub connection_id: String,
    pub grant_id: String,
    pub generation: u64,
    pub runtime_origin: String,
    pub model_origin: String,
    pub operation: ReadOperation,
    pub args: serde_json::Value,
    pub expires_at: String,
}
struct ActiveRead {
    id: String,
    context: RuntimeReadContext,
    cancel: CancellationToken,
}
#[derive(Default)]
pub(super) struct Grants {
    items: HashMap<String, ConnectorGrant>,
    active: HashMap<String, ActiveRead>,
    cancelled_runs: HashMap<(String, String), i64>,
}

pub(super) fn denied() -> AppError {
    AppError::new(
        "connector_grant_denied",
        "This read does not have a current scoped account and model-egress grant.",
    )
}
pub(super) fn timestamp(value: &str) -> AppResult<i64> {
    if value.len() > 40 {
        return Err(AppError::invalid());
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|t| t.timestamp_millis())
        .map_err(|_| AppError::invalid())
}
pub(super) fn identity(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(AppError::invalid());
    }
    Ok(())
}
fn origin(value: &str) -> AppResult<()> {
    let url = reqwest::Url::parse(value).map_err(|_| AppError::invalid())?;
    // HTTP is permitted only for the explicitly selected private Tailscale runtime/model host.
    let private_http = url.scheme() == "http"
        && matches!(
            url.host_str(),
            Some("miniforum1.tail5788db.ts.net" | "100.90.255.43")
        );
    if value.len() > 256
        || !(url.scheme() == "https" || private_http)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || url.host_str().is_none()
        || url.origin().ascii_serialization() != value
    {
        return Err(AppError::invalid());
    }
    Ok(())
}
impl GrantRequest {
    fn validate(&self, now: i64) -> AppResult<()> {
        identity(&self.workspace_id)?;
        identity(&self.device_id)?;
        validate_id(&self.connection_id)?;
        let expires = timestamp(&self.expires_at)?;
        if !self.account_read_consented
            || !self.egress.consented
            || self.egress.retention != "ephemeral-run"
            || expires <= now
            || expires > now + MAX_GRANT_SECONDS * 1000
            || self.operations.is_empty()
            || self.operations.len() > 2
        {
            return Err(denied());
        }
        origin(&self.egress.runtime_origin)?;
        origin(&self.egress.model_origin)?;
        let categories: Vec<_> = self
            .operations
            .iter()
            .map(|op| op.category().to_owned())
            .collect();
        if categories.len() != self.egress.data_categories.len()
            || categories
                .iter()
                .any(|c| !self.egress.data_categories.contains(c))
            || self
                .operations
                .iter()
                .enumerate()
                .any(|(i, op)| self.operations[..i].contains(op))
        {
            return Err(denied());
        }
        Ok(())
    }
}
impl Grants {
    fn expire(&mut self, now: i64) {
        let expired: Vec<_> = self
            .items
            .iter()
            .filter(|(_, g)| timestamp(&g.scope.expires_at).unwrap_or(0) <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.revoke(&id);
        }
        self.cancelled_runs.retain(|_, expiry| *expiry > now);
        self.active.retain(|_, read| {
            if timestamp(&read.context.expires_at).unwrap_or(0) <= now {
                read.cancel.cancel();
                false
            } else {
                true
            }
        });
    }
    pub fn register(
        &mut self,
        request: GrantRequest,
        status: &ConnectorStatus,
        now: i64,
    ) -> AppResult<ConnectorGrant> {
        self.expire(now);
        request.validate(now)?;
        if status.id != request.connection_id
            || status.provider != "google"
            || request
                .operations
                .iter()
                .any(|op| !op.scope_allowed(&status.scopes))
        {
            return Err(denied());
        }
        if self.items.len() >= MAX_GRANTS {
            return Err(AppError::new(
                "connector_limit",
                "Too many active account grants.",
            ));
        }
        let grant = ConnectorGrant {
            grant_id: uuid::Uuid::new_v4().to_string(),
            generation: 1,
            scope: request,
        };
        self.items.insert(grant.grant_id.clone(), grant.clone());
        Ok(grant)
    }
    pub fn list(&mut self, now: i64) -> Vec<ConnectorGrant> {
        self.expire(now);
        let mut grants: Vec<_> = self.items.values().cloned().collect();
        grants.sort_by(|a, b| a.grant_id.cmp(&b.grant_id));
        grants
    }
    pub fn check(&mut self, context: &RuntimeReadContext, now: i64) -> AppResult<()> {
        self.expire(now);
        identity(&context.workspace_id)?;
        identity(&context.run_id)?;
        identity(&context.device_id)?;
        validate_id(&context.connection_id)?;
        validate_id(&context.grant_id)?;
        let expires = timestamp(&context.expires_at)?;
        let grant = self.items.get(&context.grant_id).ok_or_else(denied)?;
        let scope = &grant.scope;
        if context.generation != grant.generation
            || scope.workspace_id != context.workspace_id
            || scope.device_id != context.device_id
            || scope.connection_id != context.connection_id
            || scope.egress.runtime_origin != context.runtime_origin
            || scope.egress.model_origin != context.model_origin
            || !scope.operations.contains(&context.operation)
            || !scope
                .egress
                .data_categories
                .iter()
                .any(|c| c == context.operation.category())
            || !scope.account_read_consented
            || !scope.egress.consented
            || expires <= now
            || expires > now + MAX_READ_SECONDS * 1000
            || expires > timestamp(&scope.expires_at)?
            || self
                .cancelled_runs
                .contains_key(&(context.workspace_id.clone(), context.run_id.clone()))
        {
            return Err(denied());
        }
        Ok(())
    }
    pub fn begin(
        &mut self,
        context: &RuntimeReadContext,
        now: i64,
    ) -> AppResult<(String, CancellationToken)> {
        self.check(context, now)?;
        if self.active.contains_key(&context.connection_id) {
            return Err(AppError::new(
                "connector_pending",
                "This connection already has an active read.",
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        self.active.insert(
            context.connection_id.clone(),
            ActiveRead {
                id: id.clone(),
                context: context.clone(),
                cancel: cancel.clone(),
            },
        );
        Ok((id, cancel))
    }
    pub fn check_read(
        &mut self,
        context: &RuntimeReadContext,
        id: &str,
        now: i64,
    ) -> AppResult<()> {
        self.check(context, now)?;
        if !self
            .active
            .get(&context.connection_id)
            .is_some_and(|r| r.id == id && !r.cancel.is_cancelled())
        {
            return Err(lifecycle::stale_error());
        }
        Ok(())
    }
    pub fn finish(&mut self, connection: &str, id: &str) {
        if self
            .active
            .get(connection)
            .is_some_and(|read| read.id == id)
        {
            self.active.remove(connection);
        }
    }
    pub fn revoke(&mut self, id: &str) {
        self.items.remove(id);
        self.active.retain(|_, read| {
            if read.context.grant_id == id {
                read.cancel.cancel();
                false
            } else {
                true
            }
        });
    }
    pub fn revoke_connection(&mut self, connection: &str) {
        self.items
            .retain(|_, grant| grant.scope.connection_id != connection);
        if let Some(read) = self.active.remove(connection) {
            read.cancel.cancel();
        }
    }
    pub fn cancel_run(&mut self, workspace: &str, run: &str, now: i64) -> AppResult<()> {
        identity(workspace)?;
        identity(run)?;
        self.expire(now);
        // Fail closed on saturation: revoke the workspace rather than evict a live fence.
        if self.cancelled_runs.len() >= MAX_CANCELLED_RUNS {
            let ids: Vec<_> = self
                .items
                .iter()
                .filter(|(_, g)| g.scope.workspace_id == workspace)
                .map(|(id, _)| id.clone())
                .collect();
            for id in ids {
                self.revoke(&id);
            }
            return Err(AppError::new(
                "connector_limit",
                "Workspace account grants were revoked because the cancellation limit was reached.",
            ));
        }
        self.cancelled_runs.insert(
            (workspace.into(), run.into()),
            now + MAX_GRANT_SECONDS * 1000,
        );
        self.active.retain(|_, read| {
            if read.context.workspace_id == workspace && read.context.run_id == run {
                read.cancel.cancel();
                false
            } else {
                true
            }
        });
        Ok(())
    }
}
