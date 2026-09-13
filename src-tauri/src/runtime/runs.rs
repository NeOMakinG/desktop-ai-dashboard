use super::{dto::*, session, storage::StoredRequest, transport};
use crate::{
    core::{Core, NativeState},
    types::*,
    validation,
};
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

/// Called only after the app-owned process has been reaped during explicit Quit.
/// This is local interruption, never a forged upstream cancellation receipt.
pub fn interrupt_for_quit(
    core: &Core,
    workspace_id: &str,
    request_id: &str,
) -> AppResult<ChatWorkspace> {
    validation::id(workspace_id)?;
    validation::id(request_id)?;
    let workspace = core.store.workspace(workspace_id)?;
    if !workspace
        .messages
        .iter()
        .any(|m| m.request_id.as_deref() == Some(request_id))
    {
        return Err(AppError::missing());
    }
    let mut request = core.store.runtime_request(request_id)?;
    if request
        .as_ref()
        .is_some_and(|r| r.input.workspace_id != workspace_id)
    {
        return Err(AppError::missing());
    }
    let tx = core.store.db.unchecked_transaction()?;
    let changed = tx.execute("UPDATE messages SET status='error',content='Reply interrupted when Forma quit. Previously sent provider work and external actions were not rolled back.' WHERE workspace_id=?1 AND request_id=?2 AND status='pending'", rusqlite::params![workspace_id,request_id])?;
    if changed > 0 {
        if let Some(request) = &mut request {
            request.cancel_requested = true;
            request.progress.state = "interrupted".into();
            tx.execute(
                "UPDATE runtime_requests SET value=?1 WHERE request_id=?2",
                rusqlite::params![super::storage::encode(request)?, request_id],
            )?;
        }
    }
    tx.commit()?;
    core.store.workspace(workspace_id)
}

pub fn start(
    core: &mut Core,
    workspace_id: &str,
    content: &str,
    request_id: &str,
) -> AppResult<ChatWorkspace> {
    let pending: u32 = core.store.db.query_row(
        "SELECT count(*) FROM messages WHERE status='pending'",
        [],
        |r| r.get(0),
    )?;
    if pending >= 8 {
        return Err(AppError::new(
            "busy",
            "Finish or reconcile pending runs before admitting more work.",
        ));
    }
    let session = session(core, true)?;
    let workspace = core.store.runtime_workspace(workspace_id)?;
    let status = core.store.runtime_status()?;
    if workspace.model_id.is_empty()
        || !status
            .models
            .iter()
            .any(|m| m.available && m.id == workspace.model_id)
    {
        return Err(AppError::new(
            "model_unavailable",
            "Choose an available Hermes model for this workspace.",
        ));
    }
    validation::text(content, 16_000, false)?;
    let history = core.store.workspace(workspace_id)?;
    let import_history: Vec<_> = if workspace.remote_initialized {
        Vec::new()
    } else {
        history
            .messages
            .iter()
            .filter(|m| m.status == "complete")
            .map(|m| HistoryMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect()
    };
    if import_history.len() > 100
        || import_history
            .iter()
            .any(|message| message.content.chars().count() > 16_000)
        || serde_json::to_vec(&import_history)
            .map_err(|_| AppError::invalid())?
            .len()
            > 128 * 1024
    {
        return Err(AppError::new(
            "context_limit",
            "This workspace is too large to import into Hermes. Existing history is unchanged.",
        ));
    }
    let caps = status.capabilities.ok_or_else(transport::not_ready)?;
    let budgets = Budgets {
        max_iterations: caps.limits.max_iterations.min(8),
        max_tool_calls: caps.limits.max_tool_calls.min(12),
        max_output_tokens: caps.limits.max_output_tokens.min(4096),
        max_duration_seconds: caps.limits.max_duration_seconds.min(180),
    };
    if budgets.max_iterations == 0
        || budgets.max_output_tokens == 0
        || budgets.max_duration_seconds == 0
    {
        return Err(transport::protocol());
    }
    let request = StoredRequest {
        input: RunInput {
            workspace_id: workspace_id.to_owned(),
            message: content.to_owned(),
            model_id: workspace.model_id,
            grant_refs: Vec::new(),
            interface_ids: Vec::new(),
            budgets,
            import_history,
        },
        config_generation: session.generation,
        workspace_generation: workspace.generation,
        dispatched: false,
        cancel_requested: false,
        progress: RuntimeProgress {
            workspace_id: workspace_id.to_owned(),
            request_id: request_id.to_owned(),
            generation: session.generation,
            run: None,
            events: Vec::new(),
            cursor: 0,
            state: "queued".into(),
        },
    };
    core.store.start_captured(
        workspace_id,
        content,
        request_id,
        session.generation,
        Some(&request),
    )?;
    core.store.workspace(workspace_id)
}
fn validate_run(request: &StoredRequest, run: &Run) -> AppResult<()> {
    if run.id != request.progress.request_id
        || run.workspace_id != request.input.workspace_id
        || run.model_id != request.input.model_id
        || run.origin != "operator"
        || run.schedule_id.is_some()
        || run.hermes_revision != "349e6611a1c5d846a865368dd6c386b78edd1a54"
        || run.last_seq > validation::MAX_VERSION
    {
        return Err(transport::protocol());
    }
    if run
        .final_message
        .as_ref()
        .is_some_and(|text| text.len() > 100_000)
    {
        return Err(transport::protocol());
    }
    Ok(())
}
fn state_name(state: &RunState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "uncertain".into())
}
pub(super) fn apply_poll(
    core: &mut Core,
    request_id: &str,
    poll: RunPoll,
) -> AppResult<(StoredRequest, Vec<ToolRequest>, bool)> {
    let mut request = core
        .store
        .runtime_request(request_id)?
        .ok_or_else(AppError::missing)?;
    core.store.runtime_current(&request)?;
    validate_run(&request, &poll.run)?;
    if poll.events.len() > 200
        || poll.tool_requests.len() > 12
        || poll.next_after < request.progress.cursor
        || poll.next_after > poll.run.last_seq
    {
        return Err(transport::protocol());
    }
    let mut cursor = request.progress.cursor;
    for event in &poll.events {
        if event.run_id != request_id
            || event.seq != cursor + 1
            || serde_json::to_vec(event)
                .map_err(|_| transport::protocol())?
                .len()
                > 100_000
        {
            return Err(transport::protocol());
        }
        cursor = event.seq;
    }
    if cursor != poll.next_after || (!poll.has_more && cursor != poll.run.last_seq) {
        return Err(transport::protocol());
    }
    request.progress.cursor = cursor;
    request.progress.state = state_name(&poll.run.state);
    request.progress.events.extend(poll.events);
    if request.progress.events.len() > 200 {
        request
            .progress
            .events
            .drain(..request.progress.events.len() - 200);
    }
    request.progress.run = Some(poll.run.clone());
    let mut bytes = serde_json::to_vec(&request.progress)
        .map_err(|_| transport::protocol())?
        .len();
    let mut remove = 0;
    for event in &request.progress.events {
        if bytes <= 384 * 1024 {
            break;
        }
        bytes = bytes.saturating_sub(
            serde_json::to_vec(event)
                .map_err(|_| transport::protocol())?
                .len(),
        );
        remove += 1;
    }
    if bytes > 384 * 1024 {
        return Err(transport::protocol());
    }
    request.progress.events.drain(..remove);
    // Record the authenticated admission before any terminal local message: a crash
    // after final persistence must not cause a second history import on the next turn.
    let mut workspace = core.store.runtime_workspace(&request.input.workspace_id)?;
    workspace.remote_initialized = true;
    core.store.runtime_save_workspace(&workspace)?;
    let complete = poll.run.state.terminal() && !poll.has_more;
    if complete {
        let (status, text) = match poll.run.state {
            RunState::Succeeded => (
                "complete",
                poll.run
                    .final_message
                    .as_deref()
                    .ok_or_else(transport::protocol)?,
            ),
            RunState::Cancelled => (
                "cancelled",
                "Hermes run cancelled. Remote actions are not rolled back.",
            ),
            RunState::Blocked => (
                "error",
                "Hermes run blocked: its required runtime, device or grant was unavailable.",
            ),
            RunState::Interrupted => (
                "error",
                "Hermes run interrupted. Partial output was not published as success.",
            ),
            _ => (
                "error",
                "Hermes run failed. No direct provider fallback was used.",
            ),
        };
        core.store.runtime_finish(&request, status, text)?;
    } else {
        core.store.runtime_save_request(&request)?;
    }
    Ok((request, poll.tool_requests, complete))
}

pub async fn complete(
    app: &AppHandle,
    state: &NativeState,
    workspace_id: String,
    request_id: String,
) -> AppResult<ChatWorkspace> {
    let recovered_cancel = {
        let core = state.lock()?;
        let request = core
            .store
            .runtime_request(&request_id)?
            .ok_or_else(AppError::missing)?;
        if request.input.workspace_id != workspace_id {
            return Err(AppError::missing());
        }
        core.store.runtime_pending(&request)?;
        request.cancel_requested
    };
    if recovered_cancel {
        return cancel(app, state, &workspace_id, &request_id).await;
    }
    let (session, token, mut request) = {
        let mut core = state.lock()?;
        let request = core
            .store
            .runtime_request(&request_id)?
            .ok_or_else(AppError::missing)?;
        if request.input.workspace_id != workspace_id {
            return Err(AppError::missing());
        }
        core.store.runtime_current(&request)?;
        if core.runtime_requests.contains_key(&request_id) {
            return Err(AppError::new(
                "busy",
                "This Hermes run is already being reconciled.",
            ));
        }
        let session = session(&core, false)?;
        let token = CancellationToken::new();
        core.runtime_requests
            .insert(request_id.clone(), token.clone());
        (session, token, request)
    };
    let result = async {
        if !request.dispatched {
            // Commit dispatch intent before network. Every restart uses this same immutable
            // request body and UUID; GET reconciliation never manufactures a second run.
            request.dispatched = true;
            { let core = state.lock()?; core.store.runtime_current(&request)?; super::require_binding(&core.store.runtime_status()?, true)?; core.store.runtime_own_workspace(&workspace_id, session.generation)?; core.store.runtime_save_request(&request)?; }
            let run: Run = tokio::select! { biased; _ = token.cancelled() => return Err(AppError::stale()), result = session.post("/v1/runs", &request_id, &request.input) => result? };
            validate_run(&request, &run)?;
            { let core = state.lock()?; core.store.runtime_current(&request)?; let mut workspace = core.store.runtime_workspace(&workspace_id)?; workspace.remote_initialized = true; core.store.runtime_save_workspace(&workspace)?; }
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(210);
        loop {
            if tokio::time::Instant::now() >= deadline { return Err(transport::network()); }
            let path = format!("/v1/runs/{request_id}?after={}&limit=200", request.progress.cursor);
            let poll: AppResult<RunPoll> = tokio::select! { biased; _ = token.cancelled() => return Err(AppError::stale()), result = session.get(&path) => result };
            let poll = match poll {
                Ok(poll) => poll,
                Err(error) if error.code == "runtime_not_found" => {
                    // Admission may have been lost before receipt. Safe replay is exact,
                    // durable and server-idempotent, never a new UUID or changed body.
                    { let core = state.lock()?; core.store.runtime_current(&request)?; super::require_binding(&core.store.runtime_status()?, true)?; core.store.runtime_own_workspace(&workspace_id, session.generation)?; }
                    let run: Run = tokio::select! { biased; _ = token.cancelled() => return Err(AppError::stale()), result = session.post("/v1/runs", &request_id, &request.input) => result? };
                    validate_run(&request, &run)?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let has_more = poll.has_more;
            let (next, tools, finished) = {
                let mut core = state.lock()?;
                let result = apply_poll(&mut core, &request_id, poll)?;
                let mut workspace = core.store.runtime_workspace(&workspace_id)?;
                workspace.remote_initialized = true; core.store.runtime_save_workspace(&workspace)?;
                result
            };
            request = next;
            if finished { return state.lock()?.store.workspace(&workspace_id); }
            for tool in tools {
                tokio::select! { biased; _ = token.cancelled() => return Err(AppError::stale()), result = dispatch_tool(app, state, &session, &request, tool) => result? };
            }
            if !has_more { tokio::select! { biased; _ = token.cancelled() => return Err(AppError::stale()), _ = tokio::time::sleep(std::time::Duration::from_millis(750)) => () }; }
        }
    }.await;
    let mut core = state.lock()?;
    core.runtime_requests.remove(&request_id);
    if result.is_err() {
        if let Some(mut request) = core.store.runtime_request(&request_id)? {
            if core.store.runtime_current(&request).is_ok() {
                request.progress.state = "uncertain".into();
                core.store.runtime_save_request(&request)?;
            }
        }
    }
    result
}

pub(super) fn apply_cancel_receipt(
    core: &Core,
    request: &mut StoredRequest,
    receipt: CancelReceipt,
) -> AppResult<Option<Run>> {
    core.store.runtime_pending(request)?;
    if !request.cancel_requested
        || receipt.id != request.progress.request_id
        || receipt.admitted != receipt.run.is_some()
    {
        return Err(transport::protocol());
    }
    if let Some(run) = &receipt.run {
        validate_run(request, run)?;
        if receipt.state != run.state {
            return Err(transport::protocol());
        }
        // Admission may have succeeded while its original response was lost. Even a
        // still-cancelling receipt proves the server consumed the one-time import.
        let mut workspace = core.store.runtime_workspace(&request.input.workspace_id)?;
        workspace.remote_initialized = true;
        core.store.runtime_save_workspace(&workspace)?;
        request.progress.state = state_name(&run.state);
        request.progress.run = Some(run.clone());
    } else {
        if receipt.state != RunState::Cancelled {
            return Err(transport::protocol());
        }
        request.progress.state = "cancelled".into();
    }
    core.store.runtime_save_request(request)?;
    Ok(receipt.run)
}

pub(super) fn terminal_workspace(
    core: &Core,
    workspace_id: &str,
    request_id: &str,
) -> AppResult<Option<ChatWorkspace>> {
    let workspace = core.store.workspace(workspace_id)?;
    if workspace.messages.iter().any(|message| {
        message.request_id.as_deref() == Some(request_id) && message.status == "pending"
    }) {
        Ok(None)
    } else {
        Ok(Some(workspace))
    }
}

pub async fn cancel(
    app: &AppHandle,
    state: &NativeState,
    workspace_id: &str,
    request_id: &str,
) -> AppResult<ChatWorkspace> {
    let (session, mut request) = {
        let mut core = state.lock()?;
        let mut request = core
            .store
            .runtime_request(request_id)?
            .ok_or_else(AppError::missing)?;
        if request.input.workspace_id != workspace_id {
            return Err(AppError::missing());
        }
        // Completion can win the race with Stop. Return the already durable terminal
        // snapshot rather than stranding the renderer in a failed-cancel retry loop.
        if let Some(workspace) = terminal_workspace(&core, workspace_id, request_id)? {
            return Ok(workspace);
        }
        core.store.runtime_pending(&request)?;
        let session = session(&core, false)?;
        if request.config_generation != session.generation {
            return Err(AppError::stale());
        }
        request.cancel_requested = true;
        core.store.runtime_save_request(&request)?;
        if let Some(token) = core.runtime_requests.remove(request_id) {
            token.cancel();
        }
        (session, request)
    };
    crate::connectors::cancel_runtime_run(app, workspace_id, request_id)?;
    if request.dispatched {
        let receipt: CancelReceipt = session
            .post(
                &format!("/v1/runs/{request_id}/cancel"),
                request_id,
                &json!({}),
            )
            .await?;
        let admitted = {
            let core = state.lock()?;
            if let Some(workspace) = terminal_workspace(&core, workspace_id, request_id)? {
                return Ok(workspace);
            }
            apply_cancel_receipt(&core, &mut request, receipt)?
        };
        if let Some(mut run) = admitted {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
            while !run.state.terminal() {
                if tokio::time::Instant::now() >= deadline {
                    return Err(transport::network());
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let poll: RunPoll = session
                    .get(&format!(
                        "/v1/runs/{request_id}?after={}&limit=1",
                        run.last_seq
                    ))
                    .await?;
                validate_run(&request, &poll.run)?;
                run = poll.run;
            }
            request.progress.state = state_name(&run.state);
            request.progress.run = Some(run);
        }
    } else {
        request.progress.state = "cancelled".into();
    }
    let core = state.lock()?;
    if core.store.runtime_config()?.config.generation != session.generation {
        return Err(AppError::stale());
    }
    if terminal_workspace(&core, workspace_id, request_id)?.is_none() {
        core.store.runtime_finish_cancel(&request)?;
    }
    core.store.workspace(workspace_id)
}

pub async fn delete_workspace(
    app: &AppHandle,
    state: &NativeState,
    workspace_id: &str,
) -> AppResult<()> {
    validation::id(workspace_id)?;
    let requests: Vec<String> = {
        let core = state.lock()?;
        core.store.workspace(workspace_id)?;
        let mut statement = core.store.db.prepare("SELECT DISTINCT request_id FROM messages WHERE workspace_id=?1 AND status='pending' AND request_id IN (SELECT request_id FROM runtime_requests)")?;
        let rows = statement
            .query_map([workspace_id], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for request in requests {
        cancel(app, state, workspace_id, &request).await?;
    }
    let remote = {
        let core = state.lock()?;
        core.store
            .runtime_deletion_owner(workspace_id)?
            .map(|owner| session(&core, false).map(|session| (session, owner)))
            .transpose()?
    };
    if let Some((session, owner)) = remote {
        let deleted: Deleted = session
            .post(
                &format!("/v1/workspaces/{workspace_id}/delete"),
                workspace_id,
                &json!({}),
            )
            .await?;
        if deleted.id != workspace_id {
            return Err(transport::protocol());
        }
        state.lock()?.store.runtime_acknowledge_deletion(
            workspace_id,
            &owner,
            session.generation,
        )?;
    }
    let mut core = state.lock()?;
    // Another historical binding must be fenced as well; a receipt from the current
    // server is not evidence that schedules at a former origin/library have stopped.
    if !core.store.runtime_owners(workspace_id)?.is_empty() {
        return Err(AppError::new("runtime_ownership_unresolved", "Other runtime bindings still own this workspace. Reconnect and verify each original runtime before deleting; local data was kept."));
    }
    // Store deletion atomically purges local prompt/progress copies; shared Interfaces
    // live remotely and have an independent lifecycle.
    core.delete(workspace_id)
}

async fn dispatch_tool(
    app: &AppHandle,
    state: &NativeState,
    session: &transport::Session,
    request: &StoredRequest,
    tool: ToolRequest,
) -> AppResult<()> {
    use crate::connectors::{ReadOperation, RuntimeReadContext};
    let caps = {
        let core = state.lock()?;
        core.store.runtime_current(request)?;
        core.store
            .runtime_status()?
            .capabilities
            .ok_or_else(transport::not_ready)?
    };
    // Composio connection tools are grant-free by design: they expose only
    // host-validated connect/status prompts and never read account data, so
    // neither grant containment nor connection identity applies to them.
    // (Accepted asymmetry: Google reads stay strictly gated; prompts cannot
    // exfiltrate because they carry no account payload either way.)
    let composio_tool = matches!(
        tool.tool_name.as_str(),
        crate::connectors::composio::RUNTIME_LIST_TOOL
            | crate::connectors::composio::RUNTIME_CONNECT_TOOL
    );
    // The browser-session tool is likewise grant-free: it returns only a
    // host-gated availability status plus a loopback CDP endpoint, and the
    // hand-off itself is separately gated on the user's default-off
    // "assistant may drive the browser" preference. No account data flows.
    let browser_tool = tool.tool_name == crate::browser::RUNTIME_BROWSER_SESSION_TOOL;
    let grant_free = composio_tool || browser_tool;
    if tool.run_id != request.progress.request_id
        || tool.workspace_id != request.input.workspace_id
        || tool.device_id != caps.device_id
        || (!grant_free && !request.input.grant_refs.contains(&tool.grant_ref))
    {
        return Err(transport::protocol());
    }
    let operation = match tool.tool_name.as_str() {
        "forma_gmail_list_metadata" => Some(ReadOperation::GmailListMetadata),
        "forma_calendar_list_events" => Some(ReadOperation::CalendarListEvents),
        _ if grant_free => None,
        _ => return Err(transport::protocol()),
    };
    validation::id(&tool.id)?;
    if !grant_free {
        validation::id(&tool.connection_id)?;
    }
    let path = format!("/v1/runs/{}", request.progress.request_id);
    let claim: ToolClaim = session
        .post(
            &format!("{path}/tool-claim"),
            &uuid::Uuid::new_v4().to_string(),
            &json!({"requestId":tool.id}),
        )
        .await?;
    if serde_json::to_value(&claim.request).ok() != serde_json::to_value(&tool).ok()
        && (claim.request.id != tool.id
            || claim.request.args != tool.args
            || claim.request.grant_ref != tool.grant_ref
            || claim.request.run_id != tool.run_id
            || claim.request.workspace_id != tool.workspace_id
            || claim.request.device_id != tool.device_id
            || claim.request.connection_id != tool.connection_id
            || claim.request.tool_name != tool.tool_name
            || claim.request.expires_at != tool.expires_at)
    {
        return Err(transport::protocol());
    }
    let claim_deadline = chrono::DateTime::parse_from_rfc3339(&claim.claim_expires_at)
        .map_err(|_| transport::protocol())?;
    if claim_deadline <= chrono::Utc::now()
        || claim_deadline > chrono::Utc::now() + chrono::Duration::seconds(31)
    {
        return Err(transport::protocol());
    }
    let mut delivery_context: Option<RuntimeReadContext> = None;
    let result: AppResult<Value> = match operation {
        None if browser_tool => crate::browser::runtime_browser_session(app),
        None => {
            // Native Composio work is bounded by the claim window (F2): an
            // overrun reports an honest unavailable outcome instead of
            // expiring the claim with no tool result posted. The margin keeps
            // the result post inside the claim deadline.
            let remaining = claim_deadline
                .signed_duration_since(chrono::Utc::now())
                .to_std()
                .unwrap_or_default();
            let bound = remaining.saturating_sub(std::time::Duration::from_secs(2));
            match tokio::time::timeout(
                bound,
                crate::connectors::dispatch_runtime_tool(
                    app,
                    &tool.workspace_id,
                    &tool.run_id,
                    &tool.tool_name,
                    &tool.args,
                ),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(AppError::new(
                    "composio_timeout",
                    "The connector host did not answer within the tool claim window.",
                )),
            }
        }
        Some(operation) => {
            let context = RuntimeReadContext {
                workspace_id: request.input.workspace_id.clone(),
                run_id: request.progress.request_id.clone(),
                device_id: caps.device_id,
                connection_id: tool.connection_id,
                grant_id: tool.grant_ref.id,
                generation: tool.grant_ref.generation,
                runtime_origin: session.endpoint.clone(),
                model_origin: caps.model_origin,
                operation,
                args: tool.args,
                expires_at: tool.expires_at,
            };
            let read = crate::connectors::dispatch_runtime_read(app, context.clone()).await;
            delivery_context = Some(context);
            read.and_then(|data| serde_json::to_value(&data).map_err(|_| transport::protocol()))
        }
    };
    {
        let core = state.lock()?;
        core.store.runtime_current(request)?;
    }
    if claim_deadline <= chrono::Utc::now() {
        return Err(AppError::stale());
    }
    let body = match result {
        Ok(data) => {
            if let Some(context) = &delivery_context {
                crate::connectors::validate_runtime_delivery(app, context)?;
            }
            json!({"requestId":tool.id,"claimToken":claim.claim_token,"outcome":"succeeded","data":data})
        }
        Err(_) => {
            json!({"requestId":tool.id,"claimToken":claim.claim_token,"outcome":"unavailable","error":{"code":"native_tool_unavailable","message":"The credential-owning host denied or could not perform the granted read."}})
        }
    };
    let _: Value = session
        .post(
            &format!("{path}/tool-result"),
            &uuid::Uuid::new_v4().to_string(),
            &body,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_reconciled_terminal_states_can_publish() {
        assert!(!RunState::Running.terminal());
        assert!(!RunState::WaitingForDevice.terminal());
        assert!(!RunState::Cancelling.terminal());
        assert!(RunState::Succeeded.terminal());
        assert!(RunState::Interrupted.terminal());
        assert!(RunState::Blocked.terminal());
    }
}
