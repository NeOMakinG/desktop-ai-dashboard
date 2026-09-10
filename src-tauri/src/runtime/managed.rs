use super::{
    dto::*,
    storage,
    transport::{self, Controller, Launch},
};
use crate::{
    core::{Core, NativeState},
    types::*,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};
use tauri::{AppHandle, Manager};

fn asset(root: &Path, relative: &str) -> AppResult<PathBuf> {
    let relative = Path::new(relative);
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(transport::assets());
    }
    let mut path = root.to_path_buf();
    for part in relative.components() {
        path.push(part);
        if fs::symlink_metadata(&path)
            .map_err(|_| transport::assets())?
            .file_type()
            .is_symlink()
        {
            return Err(transport::assets());
        }
    }
    Ok(path)
}
fn prepare(root: PathBuf, profile: PathBuf) -> AppResult<Launch> {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err(AppError::new(
            "runtime_platform",
            "Managed Hermes is not yet supported on this platform. Execution is blocked.",
        ));
    }
    let manifest: Value = serde_json::from_slice(
        &fs::read(asset(&root, "manifest.json")?).map_err(|_| transport::assets())?,
    )
    .map_err(|_| transport::assets())?;
    if manifest["schemaVersion"] != 1
        || manifest["platform"] != "darwin-arm64"
        || manifest["hermes"]["commit"] != "349e6611a1c5d846a865368dd6c386b78edd1a54"
    {
        return Err(transport::assets());
    }
    let checksums: BTreeMap<String, String> = serde_json::from_slice(
        &fs::read(asset(&root, "checksums.json")?).map_err(|_| transport::assets())?,
    )
    .map_err(|_| transport::assets())?;
    if checksums.is_empty() || checksums.len() > 100_000 {
        return Err(transport::assets());
    }
    for (relative, expected) in &checksums {
        let path = asset(&root, relative)?;
        let mut file = fs::File::open(path).map_err(|_| transport::assets())?;
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash).map_err(|_| transport::assets())?;
        if format!("{:x}", hash.finalize()) != *expected {
            return Err(transport::assets());
        }
    }
    for required in [
        "controller/forma_runtime/managed.py",
        "python/bin/python3.12",
        "source-proof.json",
    ] {
        if !checksums.contains_key(required) {
            return Err(transport::assets());
        }
    }
    if profile.exists()
        && fs::symlink_metadata(&profile)
            .map_err(|_| AppError::storage())?
            .file_type()
            .is_symlink()
    {
        return Err(AppError::storage());
    }
    fs::create_dir_all(&profile).map_err(|_| AppError::storage())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o700))
            .map_err(|_| AppError::storage())?;
    }
    Ok(Launch {
        python: asset(&root, "python/bin/python3.12")?,
        controller: asset(&root, "controller/forma_runtime/managed.py")?,
        source: asset(&root, "source")?,
        manifest: asset(&root, "source-proof.json")?,
        profile,
    })
}
fn paths(app: &AppHandle) -> AppResult<(PathBuf, PathBuf)> {
    let root = app
        .path()
        .resource_dir()
        .map_err(|_| transport::assets())?
        .join("managed-hermes");
    #[cfg(debug_assertions)]
    let root = if root.join("manifest.json").is_file() {
        root
    } else {
        option_env!("FORMA_MANAGED_HERMES_DIR")
            .map(PathBuf::from)
            .ok_or_else(transport::assets)?
    };
    #[cfg(debug_assertions)]
    let profile = std::env::var_os("FORMA_TEST_DATA_DIR")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| app.path().app_data_dir().map_err(|_| AppError::storage()))?;
    #[cfg(not(debug_assertions))]
    let profile = app.path().app_data_dir().map_err(|_| AppError::storage())?;
    Ok((root, profile.join("managed-hermes")))
}
fn phase(core: &Core, phase: &str, message: Option<&str>) -> AppResult<()> {
    let mut saved = core.store.runtime_config()?;
    saved.state = phase.into();
    saved.message = message.map(str::to_owned);
    if phase != "ready" && phase != "needsModel" {
        saved.config.verified = false;
    }
    core.store.db.execute(
        "UPDATE runtime_config SET value=?1 WHERE id=1",
        [storage::encode(&saved)?],
    )?;
    Ok(())
}
pub async fn start(app: &AppHandle, state: &NativeState) -> AppResult<()> {
    let _gate = state.runtime_gate.lock().await;
    if state
        .runtime_stopping
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(transport::not_ready());
    }
    let old = {
        let mut core = state.lock()?;
        phase(&core, "starting", None)?;
        core.synced_model_generation = None;
        core.controller.take()
    };
    if let Some(old) = old {
        old.shutdown().await;
    }
    let result = async {
        let (root, profile) = paths(app)?;
        let launch = tokio::task::spawn_blocking(move || prepare(root, profile))
            .await
            .map_err(|_| transport::assets())??;
        if state
            .runtime_stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(transport::not_ready());
        }
        let (controller, boot) = Controller::launch(launch, &state.runtime_process).await?;
        if state
            .runtime_stopping
            .load(std::sync::atomic::Ordering::Acquire)
        {
            controller.shutdown().await;
            return Err(transport::not_ready());
        }
        {
            state.lock()?.controller = Some(controller.clone());
        }
        accept(state, &controller, boot, None).await?;
        let approved = {
            let core = state.lock()?;
            let saved = core.store.runtime_config()?;
            saved.background_approved
                && saved.model_generation == Some(core.store.provider()?.generation)
        };
        if approved && has_enabled_schedules(state).await? {
            sync_locked(state, None).await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = &result {
        let controller = state.lock()?.controller.take();
        if let Some(controller) = controller {
            controller.shutdown().await;
        }
        phase(&*state.lock()?, "error", Some(error.message))?;
    }
    result
}
async fn accept(
    state: &NativeState,
    controller: &Controller,
    boot: Value,
    generation: Option<u64>,
) -> AppResult<()> {
    if boot["protocol"] != "forma-managed-v1" {
        return Err(transport::protocol());
    }
    let caps: Capabilities =
        serde_json::from_value(boot["capabilities"].clone()).map_err(|_| transport::protocol())?;
    if boot["libraryId"] != caps.library_id || boot["deviceId"] != caps.device_id {
        return Err(transport::protocol());
    }
    let session = transport::Session::managed(
        controller.clone(),
        state.lock()?.store.runtime_config()?.config.generation,
    )?;
    let models: Items<RuntimeModel> = session.get("/v1/models").await?;
    accept_snapshot(&mut *state.lock()?, &caps, &models.items, generation)?;
    Ok(())
}
pub(super) fn accept_snapshot(
    core: &mut Core,
    caps: &Capabilities,
    models: &[RuntimeModel],
    generation: Option<u64>,
) -> AppResult<RuntimeStatus> {
    super::validate_capabilities(caps, models)?;
    let provider = core.store.provider()?;
    if generation.is_some_and(|g| provider.generation != g) {
        return Err(AppError::stale());
    }
    if generation.is_some() {
        let expected = reqwest::Url::parse(&provider.config.base_url)
            .map_err(|_| transport::protocol())?
            .origin()
            .ascii_serialization();
        if caps.model_origin != expected {
            return Err(AppError::new(
                "runtime_binding_changed",
                "Hermes did not acknowledge the approved model destination.",
            ));
        }
    }
    let mut saved = core.store.runtime_config()?;
    if saved
        .bound_library_id
        .as_ref()
        .is_some_and(|id| id != &caps.library_id)
        || saved
            .bound_device_id
            .as_ref()
            .is_some_and(|id| id != &caps.device_id)
    {
        return Err(AppError::new(
            "runtime_binding_changed",
            "Managed Hermes profile identity changed. Existing work was preserved.",
        ));
    }
    if let Some(old) = core.store.runtime_status()?.capabilities {
        if old.device_id != caps.device_id || old.library_id != caps.library_id {
            return Err(AppError::new("runtime_binding_changed", "Managed Hermes profile identity changed. Existing work was preserved and execution blocked."));
        }
    }
    saved.bound_library_id = Some(caps.library_id.clone());
    saved.bound_device_id = Some(caps.device_id.clone());
    saved.config.verified = true;
    saved.state = if caps.runtime.ready && !caps.model_origin.is_empty() {
        "ready"
    } else if caps.model_origin.is_empty() {
        "needsModel"
    } else {
        "error"
    }
    .into();
    saved.message = caps.runtime.reason.clone();
    if let Some(generation) = generation {
        saved.model_generation = Some(generation);
        core.synced_model_generation = Some(generation);
        core.store.db.execute("UPDATE runtime_workspaces SET value=json_set(value,'$.modelId',?1) WHERE coalesce(json_extract(value,'$.modelId'),'')=''", [&provider.config.model])?;
    }
    core.store.runtime_checked(&saved, caps, models)?;
    core.store.runtime_status()
}
pub async fn sync_model(state: &NativeState, models: Option<(u64, Vec<String>)>) -> AppResult<()> {
    let _gate = state.runtime_gate.lock().await;
    if state
        .runtime_stopping
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(transport::not_ready());
    }
    sync_locked(state, models).await
}
async fn sync_locked(state: &NativeState, models: Option<(u64, Vec<String>)>) -> AppResult<()> {
    let (controller, generation, mut config) = {
        let core = state.lock()?;
        let provider = core.store.provider()?;
        if models
            .as_ref()
            .is_some_and(|(generation, _)| *generation != provider.generation)
        {
            return Err(AppError::stale());
        }
        let controller = core
            .controller
            .clone()
            .filter(|c| c.alive())
            .ok_or_else(transport::not_ready)?;
        if core.synced_model_generation == Some(provider.generation) && models.is_none() {
            return Ok(());
        }
        if !provider.config.verified || provider.config.base_url.is_empty() {
            return Err(transport::not_ready());
        }
        let mut saved = core.store.runtime_config()?;
        let key = core.key(&provider)?;
        let catalog = models
            .map(|(_, models)| models)
            .unwrap_or_else(|| saved.approved_models.clone());
        if catalog.is_empty() {
            return Err(transport::not_ready());
        }
        saved.approved_models = catalog.clone();
        core.store.db.execute(
            "UPDATE runtime_config SET value=?1 WHERE id=1",
            [storage::encode(&saved)?],
        )?;
        let catalog: Vec<_> = catalog
            .into_iter()
            .map(|id| json!({"name":id,"id":id,"available":true}))
            .collect();
        (
            controller,
            provider.generation,
            json!({"providerId":"forma-native", "gatewayUrl":provider.config.base_url,
            "gatewayKey":key.as_deref().map(|v| v.as_str()).unwrap_or(""), "models":catalog,"configEpoch":saved.config_epoch}),
        )
    };
    let response = controller
        .control(json!({"op":"configureModel","modelConfig":config}))
        .await;
    if let Some(Value::String(secret)) = config.get_mut("gatewayKey") {
        use zeroize::Zeroize;
        secret.zeroize();
    }
    accept(state, &controller, response?, Some(generation)).await
}
pub fn provider_changed(core: &mut Core) -> AppResult<()> {
    let mut saved = core.store.runtime_config()?;
    saved.config_epoch = uuid::Uuid::new_v4().to_string();
    saved.model_generation = None;
    saved.background_approved = false;
    saved.approved_models.clear();
    saved.state = "needsModel".into();
    saved.message = None;
    saved.config.generation = saved
        .config
        .generation
        .checked_add(1)
        .ok_or_else(AppError::storage)?;
    core.synced_model_generation = None;
    for (_, token) in core.runtime_requests.drain() {
        token.cancel();
    }
    core.store.runtime_save_config(&saved)
}
pub async fn clear_model(state: &NativeState) -> AppResult<()> {
    let controller = state.lock()?.controller.clone();
    if let Some(controller) = controller.filter(|c| c.alive()) {
        controller
            .control(json!({"op":"configureModel","modelConfig":null}))
            .await?;
    }
    Ok(())
}
pub async fn shutdown(state: &NativeState) {
    state
        .runtime_stopping
        .store(true, std::sync::atomic::Ordering::Release);
    state.runtime_process.shutdown().await;
}
pub async fn has_enabled_schedules(state: &NativeState) -> AppResult<bool> {
    let controller = state
        .lock()?
        .controller
        .clone()
        .filter(|c| c.alive())
        .ok_or_else(transport::not_ready)?;
    let status = controller.control(json!({"op":"lifecycle"})).await?;
    let enabled = status
        .get("enabledSchedules")
        .and_then(Value::as_u64)
        .filter(|n| *n <= crate::validation::MAX_VERSION)
        .ok_or_else(transport::protocol)?;
    Ok(enabled > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_resources_never_borrow_python_or_a_user_profile() {
        let directory = tempfile::tempdir().unwrap();
        assert!(prepare(
            directory.path().join("missing"),
            directory.path().join("profile")
        )
        .is_err());
        assert!(!directory.path().join("profile").exists());
    }
    #[test]
    fn native_executable_has_no_direct_send_or_manual_runtime_ipc() {
        let host = include_str!("../lib.rs");
        for forbidden in [
            "runtime::runtime_configure",
            "runtime::runtime_disable",
            "runtime::runtime_set_route",
            "core::complete(",
        ] {
            assert!(!host.contains(forbidden), "{forbidden}");
        }
        assert!(host.contains("runtime::managed::start(&handle, &state).await"));
        assert!(host.contains("runtime::runs::complete(window.app_handle(), &state"));
        let provider = include_str!("../provider.rs");
        assert!(!provider.contains("client.post("));
        assert!(!provider.contains("/chat/completions"));
        let control = include_str!("transport.rs");
        assert!(!control.contains("reqwest::Client"));
        assert!(control.contains(".env_clear()"));
        assert!(control.contains(".kill_on_drop(true)"));
    }
}
