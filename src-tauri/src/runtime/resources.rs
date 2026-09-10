use super::{dto::*, interfaces, session, transport};
use crate::{core::NativeState, first_party, types::*, validation};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{State, WebviewWindow};

#[derive(Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Resource {
    ListInterfaces {
        workspace_id: String,
    },
    GetInterface {
        id: String,
    },
    ListProposals {
        workspace_id: String,
    },
    GetProposal {
        id: String,
    },
    ProposeInterface {
        input: ProposalInput,
    },
    PublishInterface {
        proposal_id: String,
        expected_revision: u64,
    },
    RenameInterface {
        id: String,
        expected_revision: u64,
        title: String,
    },
    InterfaceRevisions {
        id: String,
    },
    RollbackInterface {
        id: String,
        expected_revision: u64,
        target_revision: u64,
    },
    DeleteInterface {
        id: String,
        expected_revision: u64,
    },
    ListSchedules {
        workspace_id: String,
    },
    GetSchedule {
        id: String,
    },
    CreateSchedule {
        input: ScheduleInput,
    },
    EnableSchedule {
        id: String,
        expected_version: u64,
        consent: ScheduleConsent,
    },
    PauseSchedule {
        id: String,
        expected_version: u64,
    },
    DeleteSchedule {
        id: String,
        expected_version: u64,
    },
    ListRuns {
        workspace_id: String,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Page<T> {
    items: Vec<T>,
    next_cursor: Option<String>,
    has_more: bool,
}
fn value<T: Serialize>(v: T) -> AppResult<Value> {
    serde_json::to_value(v).map_err(|_| transport::protocol())
}
fn version(v: u64, zero: bool) -> AppResult<()> {
    if v > validation::MAX_VERSION || v == 0 && !zero {
        Err(AppError::invalid())
    } else {
        Ok(())
    }
}
fn title(s: &str) -> AppResult<()> {
    validation::single_line(s, 640, false)?;
    if s.chars().count() > 160 {
        return Err(AppError::invalid());
    }
    Ok(())
}
fn budgets(b: &Budgets) -> AppResult<()> {
    if b.max_iterations == 0
        || b.max_iterations > 8
        || b.max_tool_calls > 12
        || b.max_output_tokens == 0
        || b.max_output_tokens > 4096
        || b.max_duration_seconds == 0
        || b.max_duration_seconds > 180
    {
        return Err(AppError::invalid());
    }
    Ok(())
}
fn grants(refs: &[GrantRef]) -> AppResult<()> {
    if refs.len() > 12 {
        return Err(AppError::invalid());
    }
    let mut ids = std::collections::HashSet::new();
    for r in refs {
        validation::id(&r.id)?;
        version(r.generation, false)?;
        if !ids.insert(&r.id) {
            return Err(AppError::invalid());
        }
    }
    Ok(())
}
fn scoped_workspace(input: &Resource) -> Option<&str> {
    match input {
        Resource::ListInterfaces { workspace_id }
        | Resource::ListProposals { workspace_id }
        | Resource::ListSchedules { workspace_id }
        | Resource::ListRuns { workspace_id } => Some(workspace_id),
        Resource::ProposeInterface { input } => Some(&input.workspace_id),
        Resource::CreateSchedule { input } => Some(&input.workspace_id),
        _ => None,
    }
}
fn interface(i: &Interface, library: &str) -> AppResult<()> {
    validation::id(&i.id)?;
    version(i.revision, false)?;
    title(&i.title)?;
    if i.library_id != library {
        return Err(transport::protocol());
    }
    interfaces::validate(&i.spec)
}
pub(super) async fn pages<T: DeserializeOwned + Serialize>(
    s: &transport::Session,
    path: &str,
) -> AppResult<Vec<T>> {
    let mut result = Vec::new();
    let mut cursor = None;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..4 {
        let separator = if path.contains('?') { '&' } else { '?' };
        let url = format!(
            "{path}{separator}limit=50{}",
            cursor
                .as_ref()
                .map(|c| format!("&cursor={c}"))
                .unwrap_or_default()
        );
        let page: Page<T> = s.get(&url).await?;
        if page.items.len() > 50 {
            return Err(transport::protocol());
        }
        result.extend(page.items);
        if serde_json::to_vec(&result)
            .map_err(|_| transport::protocol())?
            .len()
            > 512 * 1024
        {
            return Err(AppError::new(
                "runtime_limit",
                "The runtime list exceeds the desktop's bounded view. Narrow the workspace scope.",
            ));
        }
        if !page.has_more {
            return Ok(result);
        }
        let next = page.next_cursor.ok_or_else(transport::protocol)?;
        if next.is_empty()
            || next.len() > 12
            || !next.bytes().all(|b| b.is_ascii_digit())
            || !seen.insert(next.clone())
        {
            return Err(transport::protocol());
        }
        cursor = Some(next);
    }
    Err(AppError::new(
        "runtime_limit",
        "More than 200 runtime objects match. This bounded view was not silently truncated.",
    ))
}

pub(super) fn authorize_resource<'a>(
    status: &'a RuntimeStatus,
    input: &Resource,
) -> AppResult<&'a Capabilities> {
    super::require_binding(status, matches!(input, Resource::EnableSchedule { .. }))
}

#[tauri::command]
pub async fn runtime_resource(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    input: Resource,
) -> AppResult<Value> {
    first_party(&window)?;
    let workspace = scoped_workspace(&input).map(str::to_owned);
    let (session, library) = {
        let core = state.lock()?;
        if let Some(id) = &workspace {
            core.store.workspace(id)?;
        }
        let status = core.store.runtime_status()?;
        let caps = authorize_resource(&status, &input)?;
        (session(&core, false)?, caps.library_id.clone())
    };
    let key = uuid::Uuid::new_v4().to_string();
    let result = match input {
        Resource::ListInterfaces { workspace_id } => {
            let items: Vec<Interface> = pages(
                &session,
                &format!("/v1/interfaces?workspaceId={workspace_id}"),
            )
            .await?;
            for item in &items {
                validation::id(&item.id)?;
                title(&item.title)?;
                if item.library_id != library || !item.spec.is_null() {
                    return Err(transport::protocol());
                }
            }
            value(items)?
        }
        Resource::GetInterface { id } => {
            validation::id(&id)?;
            let i: Interface = session.get(&format!("/v1/interfaces/{id}")).await?;
            if i.id != id {
                return Err(transport::protocol());
            }
            interface(&i, &library)?;
            value(i)?
        }
        Resource::ListProposals { workspace_id } => {
            let items: Vec<Proposal> = pages(
                &session,
                &format!("/v1/interfaces/proposals?workspaceId={workspace_id}"),
            )
            .await?;
            for item in &items {
                if item.workspace_id != workspace_id || !item.spec.is_null() {
                    return Err(transport::protocol());
                }
                validation::id(&item.id)?;
                title(&item.title)?;
            }
            value(items)?
        }
        Resource::GetProposal { id } => {
            validation::id(&id)?;
            let p: Proposal = session
                .get(&format!("/v1/interfaces/proposals/{id}"))
                .await?;
            if p.id != id {
                return Err(transport::protocol());
            }
            interfaces::validate(&p.spec)?;
            value(p)?
        }
        Resource::ProposeInterface { input } => {
            if let Some(id) = &input.interface_id {
                validation::id(id)?;
            }
            version(input.expected_revision, true)?;
            title(&input.title)?;
            interfaces::validate(&input.spec)?;
            state
                .lock()?
                .store
                .runtime_own_workspace(&input.workspace_id, session.generation)?;
            let p: Proposal = session.post("/v1/interfaces/propose", &key, &input).await?;
            if p.workspace_id != input.workspace_id {
                return Err(transport::protocol());
            }
            interfaces::validate(&p.spec)?;
            value(p)?
        }
        Resource::PublishInterface {
            proposal_id,
            expected_revision,
        } => {
            validation::id(&proposal_id)?;
            version(expected_revision, true)?;
            let i: Interface = session
                .post(
                    "/v1/interfaces/publish",
                    &key,
                    &json!({"proposalId":proposal_id,"expectedRevision":expected_revision}),
                )
                .await?;
            interface(&i, &library)?;
            value(i)?
        }
        Resource::RenameInterface {
            id,
            expected_revision,
            title: name,
        } => {
            validation::id(&id)?;
            version(expected_revision, false)?;
            title(&name)?;
            let i: Interface = session
                .post(
                    &format!("/v1/interfaces/{id}/rename"),
                    &key,
                    &json!({"expectedRevision":expected_revision,"title":name}),
                )
                .await?;
            if i.id != id {
                return Err(transport::protocol());
            }
            interface(&i, &library)?;
            value(i)?
        }
        Resource::InterfaceRevisions { id } => {
            validation::id(&id)?;
            let items: Vec<InterfaceRevision> =
                pages(&session, &format!("/v1/interfaces/{id}/revisions")).await?;
            for item in &items {
                if item.interface_id != id {
                    return Err(transport::protocol());
                }
                if !item.spec.is_null() {
                    interfaces::validate(&item.spec)?;
                }
            }
            value(items)?
        }
        Resource::RollbackInterface {
            id,
            expected_revision,
            target_revision,
        } => {
            validation::id(&id)?;
            version(expected_revision, false)?;
            version(target_revision, false)?;
            let i: Interface = session
                .post(
                    &format!("/v1/interfaces/{id}/rollback"),
                    &key,
                    &json!({"expectedRevision":expected_revision,"targetRevision":target_revision}),
                )
                .await?;
            if i.id != id {
                return Err(transport::protocol());
            }
            interface(&i, &library)?;
            value(i)?
        }
        Resource::DeleteInterface {
            id,
            expected_revision,
        } => {
            validation::id(&id)?;
            version(expected_revision, false)?;
            let d: Deleted = session
                .post(
                    &format!("/v1/interfaces/{id}/delete"),
                    &key,
                    &json!({"expectedRevision":expected_revision}),
                )
                .await?;
            if d.id != id {
                return Err(transport::protocol());
            }
            value(d)?
        }
        Resource::ListSchedules { workspace_id } => {
            let items: Vec<Schedule> = pages(
                &session,
                &format!("/v1/schedules?workspaceId={workspace_id}"),
            )
            .await?;
            for item in &items {
                if item.workspace_id != workspace_id
                    || item.timezone != "UTC"
                    || !item.prompt.is_empty()
                {
                    return Err(transport::protocol());
                }
            }
            value(items)?
        }
        Resource::GetSchedule { id } => {
            validation::id(&id)?;
            let s: Schedule = session.get(&format!("/v1/schedules/{id}")).await?;
            if s.id != id || s.timezone != "UTC" {
                return Err(transport::protocol());
            }
            value(s)?
        }
        Resource::CreateSchedule { input } => {
            validation::id(&input.interface_id)?;
            version(input.expected_interface_revision, false)?;
            validation::text(&input.prompt, 16_000, false)?;
            validation::model(&input.model_id, false)?;
            if input.timezone != "UTC"
                || input.max_runs == 0
                || input.max_runs > 100
                || input.cron.len() > 128
                || input.cron.split_whitespace().count() != 5
                || !input
                    .cron
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b" */,-".contains(&b))
            {
                return Err(AppError::invalid());
            }
            chrono::DateTime::parse_from_rfc3339(&input.end_at).map_err(|_| AppError::invalid())?;
            budgets(&input.budgets)?;
            grants(&input.grant_refs)?;
            state
                .lock()?
                .store
                .runtime_own_workspace(&input.workspace_id, session.generation)?;
            let s: Schedule = session.post("/v1/schedules", &key, &input).await?;
            if s.workspace_id != input.workspace_id || s.state != "paused" || s.timezone != "UTC" {
                return Err(transport::protocol());
            }
            value(s)?
        }
        Resource::EnableSchedule {
            id,
            expected_version,
            consent,
        } => {
            validation::id(&id)?;
            version(expected_version, false)?;
            validation::model(&consent.model_id, false)?;
            grants(&consent.grant_refs)?;
            if consent.schedule_version != expected_version {
                return Err(AppError::invalid());
            }
            // Persist explicit approval before dispatch, including a lost enable receipt.
            // Startup still queries actual enabled schedules before reading a model key.
            {
                let core = state.lock()?;
                let mut saved = core.store.runtime_config()?;
                if saved.config.generation != session.generation {
                    return Err(AppError::stale());
                }
                saved.background_approved = true;
                core.store.db.execute(
                    "UPDATE runtime_config SET value=?1 WHERE id=1",
                    [super::storage::encode(&saved)?],
                )?;
            }
            // This variant exists only behind first_party, not in model/native tool dispatch.
            let s: Schedule = session
                .post(
                    &format!("/v1/schedules/{id}/enable"),
                    &key,
                    &json!({"expectedVersion":expected_version,"consent":consent}),
                )
                .await?;
            if s.id != id || s.state != "enabled" || s.timezone != "UTC" {
                return Err(transport::protocol());
            }
            value(s)?
        }
        Resource::PauseSchedule {
            id,
            expected_version,
        } => {
            validation::id(&id)?;
            version(expected_version, false)?;
            let s: Schedule = session
                .post(
                    &format!("/v1/schedules/{id}/pause"),
                    &key,
                    &json!({"expectedVersion":expected_version}),
                )
                .await?;
            if s.id != id {
                return Err(transport::protocol());
            }
            value(s)?
        }
        Resource::DeleteSchedule {
            id,
            expected_version,
        } => {
            validation::id(&id)?;
            version(expected_version, false)?;
            let d: Deleted = session
                .post(
                    &format!("/v1/schedules/{id}/delete"),
                    &key,
                    &json!({"expectedVersion":expected_version}),
                )
                .await?;
            if d.id != id {
                return Err(transport::protocol());
            }
            value(d)?
        }
        Resource::ListRuns { workspace_id } => {
            let items: Vec<Run> =
                pages(&session, &format!("/v1/runs?workspaceId={workspace_id}")).await?;
            if items
                .iter()
                .any(|r| r.workspace_id != workspace_id || r.final_message.is_some())
            {
                return Err(transport::protocol());
            }
            value(items)?
        }
    };
    let core = state.lock()?;
    if core.store.runtime_config()?.config.generation != session.generation {
        return Err(AppError::stale());
    }
    if let Some(id) = workspace {
        core.store.workspace(&id)?;
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renderer_cannot_turn_resource_ipc_into_an_http_or_model_admin_proxy() {
        for value in [
            json!({"operation":"fetch","url":"https://evil.example"}),
            json!({"operation":"listInterfaces","workspaceId":uuid::Uuid::new_v4().to_string(),"url":"http://metadata"}),
            json!({"operation":"enableSchedule","id":uuid::Uuid::new_v4().to_string(),"expectedVersion":1,"consent":{"scheduleVersion":1,"modelId":"m","grantRefs":[]},"source":"model"}),
        ] {
            assert!(serde_json::from_value::<Resource>(value).is_err());
        }
        assert!(serde_json::from_value::<ScheduleInput>(json!({"enabled":true})).is_err());
    }
}
