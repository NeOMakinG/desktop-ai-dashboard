//! All Objective-C objects stay on the main thread. No JS automation surface.
use super::*;
use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_foundation::{NSError, NSProcessInfo, NSString, NSUUID};
use objc2_web_kit::{
    WKContentRuleList, WKContentRuleListStore, WKWebView, WKWebViewConfiguration,
    WKWebsiteDataStore,
};
use tauri::{webview::NewWindowResponse, WebviewUrl, WebviewWindowBuilder};

pub(super) fn supported() -> bool {
    NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
        >= 14
}

pub(super) fn open(
    app: &AppHandle,
    state: &BrowserState,
    generation: u64,
    target: Url,
    engine_session: bool,
) -> AppResult<()> {
    // Tauri async commands run off the main thread; every WebKit gate below
    // requires it. Hop once and report failure through the state machine —
    // the body's early errors already log their gate site.
    if MainThreadMarker::new().is_none() {
        let app2 = app.clone();
        let state2 = state.clone();
        let dispatched = app.run_on_main_thread(move || {
            if open_on_main(&app2, &state2, generation, target, engine_session).is_err() {
                state2.fail(generation);
            }
        });
        return dispatched.map_err(|_| {
            super::gate(Some(app), "main_dispatch", "browser_main_dispatch_failed")
        });
    }
    open_on_main(app, state, generation, target, engine_session)
}

fn open_on_main(
    app: &AppHandle,
    state: &BrowserState,
    generation: u64,
    target: Url,
    engine_session: bool,
) -> AppResult<()> {
    if !supported() {
        return Err(super::gate(Some(app), "supported", "browser_unsupported"));
    }
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| super::gate(Some(app), "main_thread", "browser_no_main_thread"))?;
    let root = state
        .directory
        .as_deref()
        .ok_or_else(|| super::gate(Some(app), "state_directory", "browser_profile_unavailable"))?;
    let id = profile::load_or_create(root)?;
    let state = state.clone();
    let app = app.clone();
    // This is a compiled-rule cache, NOT WKWebsiteDataStore.defaultDataStore.
    let compiler = unsafe { WKContentRuleListStore::defaultStore(mtm) }
        .ok_or_else(|| super::gate(Some(&app), "rule_store", "browser_rule_store_unavailable"))?;
    let completion = RcBlock::new(move |rule: *mut WKContentRuleList, error: *mut NSError| {
        if !state.current(generation) {
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else {
            state.fail(generation);
            return;
        };
        if !error.is_null() || rule.is_null() {
            super::gate(Some(&app), "rule_compile", "browser_rule_compile_failed");
            state.fail(generation);
            return;
        }
        // WebKit guarantees the rule's lifetime for this completion; adding it retains it.
        let result = unsafe {
            build(
                &app,
                &state,
                generation,
                id,
                &*rule,
                target.clone(),
                mtm,
                engine_session,
            )
        };
        if let Err(failure) = result {
            super::gate(Some(&app), "build", failure.code);
            state.fail(generation);
        }
    });
    unsafe {
        compiler.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(&NSString::from_str("forma-owned-denial-v1")),
            Some(&NSString::from_str(&content_rules())),
            Some(&completion),
        );
    }
    Ok(())
}

/// Called only on main, after the resource blocker has successfully compiled.
unsafe fn build(
    app: &AppHandle,
    state: &BrowserState,
    generation: u64,
    id: uuid::Uuid,
    rule: &WKContentRuleList,
    target: Url,
    mtm: MainThreadMarker,
    engine_session: bool,
) -> AppResult<()> {
    if !state.current(generation) {
        return Ok(());
    }
    let config = WKWebViewConfiguration::new(mtm);
    let data_store =
        WKWebsiteDataStore::dataStoreForIdentifier(&NSUUID::from_bytes(*id.as_bytes()), mtm);
    if !data_store.isPersistent()
        || data_store.identifier().map(|x| x.as_bytes()) != Some(*id.as_bytes())
    {
        return Err(super::gate(Some(app), "data_store", "browser_data_store_mismatch"));
    }
    config.setWebsiteDataStore(&data_store);
    let controller = config.userContentController();
    controller.addContentRuleList(rule);
    // Wry preserves this controller for a supplied configuration. Verify below as well.
    let controller_identity = (&*controller as *const _) as usize;
    let navigation_state = state.clone();
    let load_state = state.clone();
    let window = WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::External(Url::parse("about:blank").map_err(|_| invalid_url())?),
    )
    .title(if engine_session {
        "Forma — Scrapling Browser (read-only snapshots)"
    } else {
        "Forma — Owned Browser (WebKit)"
    })
    .inner_size(1120.0, 820.0)
    .min_inner_size(640.0, 480.0)
    .visible(false)
    .devtools(false)
    .disable_drag_drop_handler()
    // Explicit here and in the supplied configuration; no fallback store is ever used.
    .data_store_identifier(*id.as_bytes())
    .with_webview_configuration(config)
    .on_navigation(move |url| navigation_state.permits(generation, url))
    .on_new_window(|_, _| NewWindowResponse::Deny)
    .on_download(|_, _| false)
    .on_page_load(move |_, payload| {
        if let Ok(mut s) = load_state.lock() {
            if s.generation == generation && s.secured {
                s.status.url = public_url(payload.url());
            }
        }
        load_state.emit();
    })
    .build()
    .map_err(|_| super::gate(Some(app), "window_build", "browser_window_build_failed"))?;
    let closed_state = state.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            closed_state.closed(generation, true);
        }
    });
    let checked_window = window.clone();
    let checked_state = state.clone();
    let result = window.with_webview(move |platform| {
        // Tauri owns this WKWebView pointer; with_webview guarantees main-thread access.
        let view = &*(platform.inner() as *const WKWebView);
        let actual_config = view.configuration();
        let actual_store = actual_config.websiteDataStore();
        let actual_controller = actual_config.userContentController();
        let valid = checked_state.current(generation)
            && actual_store.isPersistent()
            && actual_store.identifier().map(|x| x.as_bytes()) == Some(*id.as_bytes())
            && (&*actual_controller as *const _) as usize == controller_identity;
        if !valid {
            tauri::async_runtime::spawn(async move {
                checked_state.fail(generation);
                let _ = checked_window.destroy();
            });
            return;
        }
        // Remove Wry/Tauri's bridge and initialization scripts before ANY external load.
        // ACL grants also remain main-window-only; every custom command retains first_party.
        actual_controller.removeAllScriptMessageHandlers();
        actual_controller.removeAllUserScripts();
        if let Ok(mut s) = checked_state.lock() {
            if s.generation != generation || s.status.phase != BrowserPhase::Opening {
                drop(s);
                tauri::async_runtime::spawn(async move {
                    let _ = checked_window.destroy();
                });
                return;
            }
            s.secured = true;
            s.status.phase = BrowserPhase::Open;
            s.status.persistent = true;
            s.status.profile_id = Some(id.to_string());
            s.status.url = Some("about:blank".into());
            s.status.error = None;
        } else {
            tauri::async_runtime::spawn(async move {
                let _ = checked_window.destroy();
            });
            return;
        }
        // with_webview holds Wry's window-id lock; dispatcher calls must run after it returns.
        tauri::async_runtime::spawn(async move {
            let ready = checked_state
                .lock()
                .map(|s| s.generation == generation && s.secured)
                .unwrap_or(false);
            if !ready {
                let _ = checked_window.destroy();
                return;
            }
            if checked_window.navigate(target).is_err() {
                checked_state.fail(generation);
                let _ = checked_window.destroy();
                return;
            }
            if checked_window
                .show()
                .and_then(|_| checked_window.set_focus())
                .is_err()
            {
                if let Ok(mut s) = checked_state.lock() {
                    if s.generation == generation && s.secured {
                        s.status.error = Some(
                            "The browser is open, but its window could not be brought forward.",
                        );
                    }
                }
            }
            checked_state.emit();
        });
    });
    if result.is_err() {
        super::gate(Some(app), "webview_verify", "browser_webview_verification_failed");
        state.fail(generation);
        let _ = window.destroy();
        return Err(AppError::new("browser_unavailable", GATE_ERROR));
    }
    Ok(())
}

pub(super) async fn history(
    window: &WebviewWindow,
    state: BrowserState,
    direction: History,
) -> AppResult<()> {
    let generation = state.lock()?.generation;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let failure = || {
        AppError::new(
            "browser_navigation_failed",
            "The browser history action could not run.",
        )
    };
    window
        .with_webview(move |platform| {
            let result = (|| unsafe {
                if tx.is_closed() || std::time::Instant::now() >= deadline {
                    return Err(AppError::new(
                        "browser_navigation_failed",
                        "The browser history action timed out.",
                    ));
                }
                if !state
                    .lock()
                    .map(|s| s.generation == generation && s.secured)
                    .unwrap_or(false)
                {
                    return Err(AppError::new(
                        "browser_closed",
                        "Open the owned browser first.",
                    ));
                }
                let view = &*(platform.inner() as *const WKWebView);
                let candidate = match direction {
                    History::Back => view.backForwardList().backItem().map(|item| item.URL()),
                    History::Forward => view.backForwardList().forwardItem().map(|item| item.URL()),
                    History::Reload => view.URL(),
                };
                let url = candidate
                    .and_then(|url| url.absoluteString())
                    .and_then(|text| Url::parse(&text.to_string()).ok())
                    .ok_or_else(|| {
                        AppError::new(
                            "browser_history_empty",
                            "There is no page in that history direction.",
                        )
                    })?;
                if !state.permits(generation, &url) {
                    return Err(invalid_url());
                }
                if tx.is_closed() || std::time::Instant::now() >= deadline {
                    return Err(AppError::new(
                        "browser_navigation_failed",
                        "The browser history action timed out.",
                    ));
                }
                let navigation = match direction {
                    History::Back => view.goBack(),
                    History::Forward => view.goForward(),
                    History::Reload => view.reload(),
                };
                if navigation.is_none() {
                    return Err(AppError::new(
                        "browser_navigation_failed",
                        "The browser did not start that history action.",
                    ));
                }
                Ok(())
            })();
            let _ = tx.send(result);
        })
        .map_err(|_| failure())?;
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), rx)
        .await
        .map_err(|_| {
            AppError::new(
                "browser_navigation_failed",
                "The browser history action timed out.",
            )
        })?
        .map_err(|_| failure())?
}
