mod browser;
mod connectors;
mod core;
mod hermes;
mod provider;
mod runtime;
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
    let _gate = state.runtime_gate.lock().await;
    let provider = state.lock()?.configure(input)?;
    runtime::managed::clear_model(&state).await?;
    Ok(provider)
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
    runtime::runs::delete_workspace(window.app_handle(), &state, &workspace_id).await
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
    validation::id(&workspace_id)?;
    validation::id(&request_id)?;
    validation::text(&content, 16_000, false)?;
    state.lock()?.store.workspace(&workspace_id)?;
    let cold_catalog = {
        let core = state.lock()?;
        let saved = core.store.runtime_config()?;
        saved.approved_models.is_empty().then(|| {
            (
                core.store.provider().map(|p| p.generation),
                core.store
                    .runtime_workspace(&workspace_id)
                    .map(|w| w.model_id),
            )
        })
    };
    if let Some((generation, model)) = cold_catalog {
        core::check_selected_model(&state, &model?, generation?).await?;
    } else {
        runtime::managed::sync_model(&state, None).await?;
    }
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
    runtime::runs::complete(window.app_handle(), &state, workspace_id, request_id).await
}
#[tauri::command]
async fn cancel_message(
    window: WebviewWindow,
    state: State<'_, NativeState>,
    workspace_id: String,
    request_id: String,
) -> AppResult<ChatWorkspace> {
    first_party(&window)?;
    if state
        .runtime_stopping
        .load(std::sync::atomic::Ordering::Acquire)
    {
        state.runtime_process.shutdown().await;
        let core = state.lock()?;
        return runtime::runs::interrupt_for_quit(&core, &workspace_id, &request_id);
    }
    let hermes = state.lock()?.store.runtime_request(&request_id)?.is_some();
    if hermes {
        runtime::runs::cancel(window.app_handle(), &state, &workspace_id, &request_id).await
    } else {
        state.lock()?.cancel(&workspace_id, &request_id)
    }
}

#[derive(Default)]
struct Lifecycle {
    quitting: std::sync::atomic::AtomicBool,
    stopped: std::sync::atomic::AtomicBool,
    tray_ready: std::sync::atomic::AtomicBool,
}
fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
fn stop_and_exit(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        runtime::managed::shutdown(&app.state::<NativeState>()).await;
        app.state::<browser::BrowserState>().shutdown();
        app.state::<Lifecycle>()
            .stopped
            .store(true, std::sync::atomic::Ordering::Release);
        app.exit(0);
    });
}
fn request_quit(app: &tauri::AppHandle) {
    // Stopping owned execution never waits on WebKit or a blocking Keychain prompt.
    let native = app.state::<NativeState>();
    native
        .runtime_stopping
        .store(true, std::sync::atomic::Ordering::Release);
    native.runtime_process.request_stop();
    app.state::<Lifecycle>()
        .quitting
        .store(true, std::sync::atomic::Ordering::Release);
    match app.get_webview_window("main") {
        Some(window) if window.is_visible().unwrap_or(true) => {
            let _ = window.close();
        }
        _ => stop_and_exit(app),
    }
}
fn install_lifecycle(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    let show = MenuItem::with_id(app, "forma-show", "Show Forma", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "forma-quit", "Quit Forma", true, None::<&str>)?;
    // Preserve native Edit/Window menus and webview copy/paste shortcuts.
    // The default Quit item reaches ExitRequested; tray Quit uses the same path.
    app.set_menu(Menu::default(app)?)?;
    app.on_menu_event(|app, event| match event.id().as_ref() {
        "forma-show" => show_main(app),
        "forma-quit" => request_quit(app),
        _ => (),
    });
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = tauri::tray::TrayIconBuilder::with_id("forma-runtime")
        .menu(&menu)
        .tooltip("Forma — Hermes runs only while this app and computer are awake")
        .title("Forma");
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    if tray.build(app).is_ok() {
        app.state::<Lifecycle>()
            .tray_ready
            .store(true, std::sync::atomic::Ordering::Release);
    }
    Ok(())
}
#[tauri::command]
async fn finish_window_close(
    window: WebviewWindow,
    state: State<'_, NativeState>,
) -> AppResult<String> {
    first_party(&window)?;
    let app = window.app_handle();
    let quitting = app
        .state::<Lifecycle>()
        .quitting
        .load(std::sync::atomic::Ordering::Acquire);
    if !quitting {
        let approved = state.lock()?.store.runtime_config()?.background_approved;
        let enabled = match runtime::managed::has_enabled_schedules(&state).await {
            Ok(enabled) => enabled,
            Err(_) if !approved => false,
            Err(_) => return Err(AppError::new("runtime_close", "Schedule state is unknown. Keep Forma open or explicitly Quit Forma to stop its processes.")),
        };
        if enabled {
            if !app
                .state::<Lifecycle>()
                .tray_ready
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return Err(AppError::new("runtime_close", "Forma could not show its background status. Keep this window open or explicitly Quit Forma."));
            }
            window
                .hide()
                .map_err(|_| AppError::new("runtime_close", "Forma could not hide its window."))?;
            return Ok("hidden".into());
        }
    }
    runtime::managed::shutdown(&state).await;
    app.state::<Lifecycle>()
        .stopped
        .store(true, std::sync::atomic::Ordering::Release);
    window.destroy().map_err(|_| AppError::storage())?;
    app.exit(0);
    Ok("closed".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Read-only snapshot surface for Scrapling engine sessions. Strict
        // CSP: no scripts, no external subresources, no form submission; the
        // only navigation allowed out of a snapshot is intercepted by the
        // browser navigation gate and re-fetched through the engine.
        .register_uri_scheme_protocol(browser::engine::SNAPSHOT_SCHEME, |context, request| {
            use std::borrow::Cow;
            const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; form-action 'none'; base-uri *; frame-ancestors 'none'";
            let snapshot = context
                .app_handle()
                .try_state::<browser::BrowserState>()
                .and_then(|state| {
                    let text = request.uri().to_string();
                    reqwest::Url::parse(&text).ok().and_then(|url| {
                        let session = url.host_str()?.to_string();
                        let id = url.path().trim_start_matches('/').to_string();
                        browser::engine::snapshot_response(&state.snapshots, &session, &id)
                    })
                });
            let (status, html, content_type) = match snapshot {
                Some((html, content_type)) => (
                    tauri::http::StatusCode::OK,
                    html,
                    content_type,
                ),
                None => (
                    tauri::http::StatusCode::NOT_FOUND,
                    "<!doctype html><meta charset=\"utf-8\"><p>Snapshot unavailable.</p>".to_string(),
                    "text/html; charset=utf-8",
                ),
            };
            tauri::http::Response::builder()
                .status(status)
                .header("Content-Type", content_type)
                .header("Cache-Control", "no-store")
                .header("Referrer-Policy", "no-referrer")
                .header("X-Content-Type-Options", "nosniff")
                .header("Content-Security-Policy", CSP)
                .body(Cow::<'static, [u8]>::Owned(html.into_bytes()))
                .unwrap_or_else(|_| {
                    tauri::http::Response::builder()
                        .status(tauri::http::StatusCode::NOT_FOUND)
                        .body(Cow::<'static, [u8]>::Borrowed(&[][..]))
                        .expect("static response")
                })
        })
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
            app.manage(
                browser::BrowserState::new(
                    directory.as_ref().ok().cloned(),
                    test_directory.is_some(),
                    Some(app.handle()),
                )
                .with_app(app.handle().clone()),
            );
            app.manage(connectors::ConnectorState::new(
                directory.as_ref().ok().cloned(),
                test_directory.clone(),
            ));
            app.manage(connectors::composio::ComposioState::new(
                test_directory.clone(),
            ));
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
            app.manage(Lifecycle::default());
            install_lifecycle(app.handle())?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<NativeState>();
                let _ = runtime::managed::start(&handle, &state).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Main's draft-flush handler destroys main once safe. Its owned window must
            // not keep the process alive; destroying it does not clear WebKit storage.
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window
                    .app_handle()
                    .state::<browser::BrowserState>()
                    .shutdown();
                if let Some(browser) = window.app_handle().get_webview_window(browser::LABEL) {
                    let _ = browser.destroy();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            browser::browser_status,
            browser::browser_open,
            browser::browser_navigate,
            browser::browser_close,
            browser::browser_back,
            browser::browser_forward,
            browser::browser_reload,
            browser::browser_embedded_state,
            browser::browser_embedded_new_tab,
            browser::browser_embedded_activate_tab,
            browser::browser_embedded_close_tab,
            browser::browser_embedded_navigate,
            browser::browser_embedded_history,
            browser::browser_embedded_input,
            browser::browser_embedded_viewport,
            browser::browser_embedded_pop,
            finish_window_close,
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
            cancel_message,
            connectors::connectors_status,
            connectors::connectors_list,
            connectors::connectors_cancel,
            connectors::connectors_capabilities,
            connectors::connectors_start_google,
            connectors::connectors_refresh,
            connectors::connectors_disconnect,
            connectors::connectors_grants_list,
            connectors::connectors_grant_register,
            connectors::connectors_grant_revoke,
            connectors::connectors_run_cancel,
            connectors::connectors_read_capabilities,
            connectors::composio::composio_status,
            connectors::composio::composio_catalog,
            connectors::composio::composio_categories,
            connectors::composio::composio_start,
            connectors::composio::composio_cancel,
            connectors::composio::composio_disconnect,
            connectors::composio::composio_key_save,
            connectors::composio::composio_key_remove,
            connectors::composio::composio_prompt_dismiss,
            runtime::runtime_status,
            runtime::runtime_retry,
            runtime::runtime_check,
            runtime::runtime_workspace,
            runtime::runtime_select_model,
            runtime::runtime_progress,
            runtime::resources::runtime_resource
        ])
        .build(tauri::generate_context!())
        .expect("Unable to start Forma")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { api, .. } => {
                let lifecycle = app.state::<Lifecycle>();
                if !lifecycle.stopped.load(std::sync::atomic::Ordering::Acquire) {
                    api.prevent_exit();
                    request_quit(app);
                }
            }
            tauri::RunEvent::Exit => {
                app.state::<browser::BrowserState>().shutdown();
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_main(app),
            _ => (),
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
