mod core;
mod provider;
mod store;
mod types;
mod validation;

use crate::{
    core::{Core, NativeState},
    types::*,
};
use tauri::{Manager, State, WebviewWindow};

fn first_party(window: &WebviewWindow) -> AppResult<()> {
    let url = window
        .url()
        .map_err(|_| AppError::new("forbidden", "This window cannot access native app data."))?;
    if window.label() != "main" || !trusted_origin(&url) {
        return Err(AppError::new(
            "forbidden",
            "This window cannot access native app data.",
        ));
    }
    Ok(())
}
fn trusted_origin(url: &reqwest::Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let production =
        (url.scheme() == "tauri" && url.host_str() == Some("localhost") && url.port().is_none())
            || (matches!(url.scheme(), "http" | "https")
                && url.host_str() == Some("tauri.localhost")
                && url.port().is_none());
    #[cfg(debug_assertions)]
    let development =
        url.scheme() == "http" && url.host_str() == Some("127.0.0.1") && url.port() == Some(1420);
    #[cfg(not(debug_assertions))]
    let development = false;
    production || development
}

#[tauri::command]
async fn app_bootstrap(
    window: WebviewWindow,
    state: State<'_, NativeState>,
) -> AppResult<Bootstrap> {
    first_party(&window)?;
    let core = state.lock()?;
    Ok(Bootstrap {
        settings: core.store.settings()?,
        workspaces: core.store.summaries()?,
    })
}
#[tauri::command]
async fn save_settings(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    input: SettingsInput,
) -> AppResult<AppSettings> {
    first_party(&window)?;
    state.lock()?.store.save_settings(input)
}
#[tauri::command]
async fn configure_provider(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    input: ProviderInput,
) -> AppResult<ProviderConfig> {
    first_party(&window)?;
    state.lock()?.configure(input)
}
#[tauri::command]
async fn check_provider(
    window: WebviewWindow,
    state: State<'_, NativeState>,
) -> AppResult<ProviderCheck> {
    first_party(&window)?;
    core::check(&state).await
}
#[tauri::command]
async fn list_models(window: WebviewWindow, state: State<'_, NativeState>) -> AppResult<ModelList> {
    first_party(&window)?;
    core::list_models(&state).await
}
#[tauri::command]
async fn create_workspace(
    window: WebviewWindow,
    state: State<'_, NativeState>,
) -> AppResult<ChatWorkspace> {
    first_party(&window)?;
    state.lock()?.store.create_workspace()
}
#[tauri::command]
async fn get_workspace(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
) -> AppResult<ChatWorkspace> {
    first_party(&window)?;
    state.lock()?.store.workspace(&workspace_id)
}
#[tauri::command]
async fn update_draft(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
    draft: String,
    version: u64,
) -> AppResult<ChatWorkspace> {
    first_party(&window)?;
    state
        .lock()?
        .store
        .update_draft(&workspace_id, &draft, version)
}
#[tauri::command]
async fn rename_workspace(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
    title: String,
) -> AppResult<ChatWorkspace> {
    first_party(&window)?;
    state.lock()?.store.rename(&workspace_id, &title)
}
#[tauri::command]
async fn delete_workspace(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
) -> AppResult<()> {
    first_party(&window)?;
    state.lock()?.delete(&workspace_id)
}
#[tauri::command]
async fn start_message(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
    content: String,
    request_id: String,
) -> AppResult<ChatWorkspace> {
    first_party(&window)?;
    state.lock()?.start(&workspace_id, &content, &request_id)
}
#[tauri::command]
async fn complete_message(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
    request_id: String,
) -> AppResult<ChatWorkspace> {
    first_party(&window)?;
    core::complete(&state, workspace_id, request_id).await
}
#[tauri::command]
async fn cancel_message(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
    request_id: String,
) -> AppResult<ChatWorkspace> {
    first_party(&window)?;
    state.lock()?.cancel(&workspace_id, &request_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            #[cfg(debug_assertions)]
            let test_directory =
                std::env::var_os("FORMA_TEST_DATA_DIR").map(std::path::PathBuf::from);
            #[cfg(not(debug_assertions))]
            let test_directory: Option<std::path::PathBuf> = None;
            let directory = test_directory
                .clone()
                .map(Ok)
                .unwrap_or_else(|| app.path().app_data_dir().map_err(|_| AppError::storage()));
            let native = directory
                .and_then(|directory| store::Store::open(&directory))
                .map(|store| {
                    Core::new(
                        store,
                        Box::new(provider::OsCredentials::new(test_directory.as_deref())),
                    )
                });
            // Keep the UI available to explain recoverable storage errors; never reset the DB.
            app.manage(NativeState::new(native));
            app.set_menu(tauri::menu::Menu::default(app.handle())?)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_bootstrap,
            save_settings,
            configure_provider,
            check_provider,
            list_models,
            create_workspace,
            get_workspace,
            update_draft,
            rename_workspace,
            delete_workspace,
            start_message,
            complete_message,
            cancel_message
        ])
        .build(tauri::generate_context!())
        .expect("Unable to start Forma")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if let Some(window) = app.get_webview_window("main") {
                    // Cmd-Q follows the same frontend draft-flush path as the close button.
                    // Once the frontend destroys the last window, allow the subsequent exit.
                    api.prevent_exit();
                    let _ = window.close();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_party_origins_only() {
        assert!(trusted_origin(
            &reqwest::Url::parse("tauri://localhost/index.html").unwrap()
        ));
        assert!(trusted_origin(
            &reqwest::Url::parse("http://tauri.localhost/").unwrap()
        ));
        for url in [
            "https://example.com",
            "http://localhost:1420",
            "http://127.0.0.1:9000",
            "https://tauri.localhost.evil.com",
            "tauri://evil",
        ] {
            assert!(!trusted_origin(&reqwest::Url::parse(url).unwrap()));
        }
    }
    #[test]
    fn settings_and_provider_reject_unknown_fields() {
        assert!(serde_json::from_value::<SettingsInput>(serde_json::json!({"onboardingComplete":false,"onboardingStep":0,"displayName":"","ambientMotion":true,"apiKey":"secret"})).is_err());
        assert!(serde_json::from_value::<ProviderInput>(
            serde_json::json!({"label":"","baseUrl":"","model":"","verified":true})
        )
        .is_err());
        let settings = AppSettings {
            schema_version: 1,
            preferences: SettingsInput::default(),
            provider: ProviderConfig::default(),
        };
        let json = serde_json::to_value(settings).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["onboardingStep"], 0);
        assert!(json.get("preferences").is_none());
        assert!(json["provider"].get("apiKey").is_none());
    }
}
