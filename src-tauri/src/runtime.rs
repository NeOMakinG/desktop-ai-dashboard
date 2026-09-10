pub mod config;
pub mod dto;
mod interfaces;
pub mod managed;
pub(crate) mod resources;
pub mod runs;
pub mod storage;
pub(crate) mod transport;

use crate::{
    core::{Core, NativeState},
    first_party,
    types::*,
    validation,
};
use dto::*;
use tauri::{Manager, State, WebviewWindow};

pub fn session(core: &Core, require_ready: bool) -> AppResult<transport::Session> {
    if require_ready {
        require_binding(&core.store.runtime_status()?, true)?;
    }
    transport::Session::managed(
        core.controller.clone().ok_or_else(transport::not_ready)?,
        core.store.runtime_config()?.config.generation,
    )
}
pub(super) fn require_binding(
    status: &RuntimeStatus,
    require_ready: bool,
) -> AppResult<&Capabilities> {
    let caps = status
        .capabilities
        .as_ref()
        .ok_or_else(transport::not_ready)?;
    if !status.config.verified
        || (require_ready
            && (status.state != "ready" || !caps.runtime.ready || caps.model_origin.is_empty()))
    {
        return Err(transport::not_ready());
    }
    Ok(caps)
}
fn pending(core: &Core, workspace: Option<&str>) -> AppResult<bool> {
    Ok(core.store.db.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE status='pending' AND (?1 IS NULL OR workspace_id=?1))", [workspace], |r| r.get(0))?)
}
#[tauri::command]
pub async fn runtime_status(
    window: WebviewWindow,
    state: State<'_, NativeState>,
) -> AppResult<RuntimeStatus> {
    first_party(&window)?;
    let core = state.lock()?;
    let mut status = core.store.runtime_status()?;
    if core.controller.is_none() && status.state != "error" {
        status.state = "starting".into();
        status.verified = false;
        status.config.verified = false;
    }
    if core.controller.as_ref().is_some_and(|c| !c.alive()) {
        status.state = "error".into();
        status.verified = false;
        status.config.verified = false;
        status.message =
            Some("Hermes stopped. Retry starting Hermes; existing work is preserved.".into());
    }
    Ok(status)
}
#[tauri::command]
pub async fn runtime_retry(
    window: WebviewWindow,
    state: State<'_, NativeState>,
) -> AppResult<RuntimeStatus> {
    first_party(&window)?;
    managed::start(window.app_handle(), &state).await?;
    state.lock()?.store.runtime_status()
}
#[tauri::command]
pub async fn runtime_check(
    window: WebviewWindow,
    state: State<'_, NativeState>,
) -> AppResult<RuntimeStatus> {
    first_party(&window)?;
    crate::core::check(&state).await?;
    state.lock()?.store.runtime_status()
}
fn validate_capabilities(caps: &Capabilities, models: &[RuntimeModel]) -> AppResult<()> {
    if caps.contract_version != "forma-runtime-v1"
        || caps.runtime.kind != "hermes"
        || !caps.features.event_polling
        || caps.features.generated_code_execution
        || caps.features.live_google
        || models.len() > 200
        || caps.tools.len() > 40
        || caps.runtime.revision != "349e6611a1c5d846a865368dd6c386b78edd1a54"
    {
        return Err(transport::protocol());
    }
    validation::id(&caps.device_id)?;
    validation::id(&caps.library_id)?;
    if !caps.model_origin.is_empty() {
        validation::endpoint(&caps.model_origin)?;
    }
    for model in models {
        validation::model(&model.id, false)?;
        validation::single_line(&model.name, 200, false)?;
    }
    Ok(())
}
#[tauri::command]
pub async fn runtime_workspace(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
) -> AppResult<RuntimeWorkspace> {
    first_party(&window)?;
    state.lock()?.store.runtime_workspace(&workspace_id)
}
#[tauri::command]
pub async fn runtime_select_model(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
    model: String,
) -> AppResult<RuntimeWorkspace> {
    first_party(&window)?;
    validation::model(&model, false)?;
    let (mut workspace, provider_generation, needs_check) = {
        let core = state.lock()?;
        if pending(&core, Some(&workspace_id))? {
            return Err(AppError::new(
                "busy",
                "Finish or stop this workspace's run before changing its model.",
            ));
        }
        let provider = core.store.provider()?;
        let status = core.store.runtime_status()?;
        (
            core.store.runtime_workspace(&workspace_id)?,
            provider.generation,
            status.state != "ready"
                || !provider.config.verified
                || core.synced_model_generation != Some(provider.generation),
        )
    };
    if needs_check {
        crate::core::check_selected_model(&state, &model, provider_generation).await?;
    }
    let session = {
        let core = state.lock()?;
        if core.store.provider()?.generation != provider_generation {
            return Err(AppError::stale());
        }
        session(&core, true)?
    };
    let models: Items<RuntimeModel> = session.get("/v1/models").await?;
    if !models.items.iter().any(|m| m.id == model && m.available) {
        return Err(AppError::new(
            "model_unavailable",
            "The runtime did not accept this model.",
        ));
    }
    let core = state.lock()?;
    if core.store.runtime_config()?.config.generation != session.generation
        || core.store.runtime_workspace(&workspace_id)?.generation != workspace.generation
        || pending(&core, Some(&workspace_id))?
    {
        return Err(AppError::stale());
    }
    workspace.model_id = model;
    workspace.generation = workspace
        .generation
        .checked_add(1)
        .filter(|n| *n <= validation::MAX_VERSION)
        .ok_or_else(AppError::storage)?;
    core.store.runtime_save_workspace(&workspace)?;
    Ok(workspace)
}
#[tauri::command]
pub async fn runtime_progress(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
    request_id: String,
) -> AppResult<RuntimeProgress> {
    first_party(&window)?;
    let core = state.lock()?;
    core.store.workspace(&workspace_id)?;
    let request = core
        .store
        .runtime_request(&request_id)?
        .ok_or_else(AppError::missing)?;
    if request.input.workspace_id != workspace_id
        || request.config_generation != core.store.runtime_config()?.config.generation
    {
        return Err(AppError::stale());
    }
    Ok(request.progress)
}
#[cfg(test)]
mod tests;
