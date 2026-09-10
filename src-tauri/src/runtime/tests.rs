use super::*;
use crate::{
    provider::{credential_reference as direct_reference, Credentials},
    store::{Store, StoredProvider},
};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

#[derive(Default, Clone)]
struct Secrets {
    values: Arc<Mutex<HashMap<String, String>>>,
    reads: Arc<Mutex<Vec<String>>>,
}
impl Credentials for Secrets {
    fn read(&self, reference: &str) -> AppResult<Zeroizing<String>> {
        self.reads.lock().unwrap().push(reference.into());
        self.values
            .lock()
            .unwrap()
            .get(reference)
            .cloned()
            .map(Zeroizing::new)
            .ok_or_else(AppError::missing)
    }
    fn write(&self, reference: &str, value: &str) -> AppResult<()> {
        self.values
            .lock()
            .unwrap()
            .insert(reference.into(), value.into());
        Ok(())
    }
    fn remove(&self, reference: &str) -> AppResult<()> {
        self.values.lock().unwrap().remove(reference);
        Ok(())
    }
}
fn setup(store: Store) -> (Core, Secrets) {
    let secrets = Secrets::default();
    let mut core = Core::new(store, Box::new(secrets.clone()));
    let direct = direct_reference("https://models.example");
    secrets
        .write(&direct, "existing-direct-token-do-not-send")
        .unwrap();
    core.store
        .set_provider(&StoredProvider {
            config: ProviderConfig {
                base_url: "https://models.example".into(),
                model: "direct-model".into(),
                verified: true,
                has_key: true,
                ..Default::default()
            },
            generation: 0,
            credential: Some(direct),
        })
        .unwrap();
    core.controller = Some(transport::Controller::fixture());
    let mut config = core.store.runtime_config().unwrap();
    config.config.verified = true;
    config.state = "ready".into();
    let caps:Capabilities=serde_json::from_value(json!({"contractVersion":"forma-runtime-v1","deviceId":"11111111-1111-4111-8111-111111111111","libraryId":"22222222-2222-4222-8222-222222222222","modelOrigin":"https://models.example","runtime":{"kind":"hermes","ready":true,"revision":"349e6611a1c5d846a865368dd6c386b78edd1a54"},"features":{"eventPolling":true,"interfaces":true,"schedules":true,"nativeToolBridge":true,"generatedCodeExecution":false,"liveGoogle":false},"tools":[],"limits":{"maxIterations":8,"maxToolCalls":12,"maxOutputTokens":4096,"maxDurationSeconds":180,"maxToolResultBytes":262144}})).unwrap();
    core.store
        .runtime_checked(
            &config,
            &caps,
            &[RuntimeModel {
                id: "runtime-model".into(),
                name: "Runtime model".into(),
                available: true,
                reason: None,
            }],
        )
        .unwrap();
    secrets.reads.lock().unwrap().clear();
    (core, secrets)
}
fn memory() -> Store {
    Store::initialize(rusqlite::Connection::open_in_memory().unwrap()).unwrap()
}
fn workspace(core: &Core) -> String {
    let id = core.store.create_workspace().unwrap().summary.id;
    let mut workspace = core.store.runtime_workspace(&id).unwrap();
    workspace.route = Route::Hermes;
    workspace.model_id = "runtime-model".into();
    core.store.runtime_save_workspace(&workspace).unwrap();
    id
}
#[test]
fn first_admission_imports_prior_history_once_without_direct_credentials_or_system_prompt() {
    let (mut core, secrets) = setup(memory());
    let id = workspace(&core);
    let old = uuid::Uuid::new_v4().to_string();
    core.store.start(&id, "previous user", &old, 0).unwrap();
    core.store
        .finish(&id, &old, 0, "complete", "previous assistant")
        .unwrap();
    let other = workspace(&core);
    core.store
        .update_draft(&other, "unrelated private draft", 1)
        .unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    core.start(&id, "current exactly once", &request).unwrap();
    let captured = core.store.runtime_request(&request).unwrap().unwrap();
    assert_eq!(captured.input.import_history.len(), 2);
    assert_eq!(captured.input.message, "current exactly once");
    let body = serde_json::to_string(&captured.input).unwrap();
    assert_eq!(body.matches("current exactly once").count(), 1);
    for forbidden in [
        "system",
        "existing-direct-token-do-not-send",
        "dedicated-runtime-test-token",
        "unrelated private draft",
    ] {
        assert!(!body.contains(forbidden), "{forbidden}");
    }
    assert!(secrets
        .reads
        .lock()
        .unwrap()
        .iter()
        .all(|r| r.starts_with("runtime:")));
    assert_eq!(
        core.store.workspace(&other).unwrap().draft,
        "unrelated private draft"
    );
    assert_eq!(core.store.provider().unwrap().config.model, "direct-model");
}
#[test]
fn restart_keeps_exact_admission_and_never_reuses_request_id() {
    let temp = tempfile::tempdir().unwrap();
    let (mut core, _) = setup(Store::open(temp.path()).unwrap());
    let id = workspace(&core);
    let request = uuid::Uuid::new_v4().to_string();
    core.start(&id, "durable admission", &request).unwrap();
    let mut captured = core.store.runtime_request(&request).unwrap().unwrap();
    captured.dispatched = true;
    core.store.runtime_save_request(&captured).unwrap();
    let before = serde_json::to_string(&captured.input).unwrap();
    drop(core);
    let store = Store::open(temp.path()).unwrap();
    let recovered = store.runtime_request(&request).unwrap().unwrap();
    assert!(recovered.dispatched);
    assert_eq!(serde_json::to_string(&recovered.input).unwrap(), before);
    assert_eq!(store.workspace(&id).unwrap().messages[1].status, "pending");
    let mut store = store;
    assert!(store.start(&id, "duplicate", &request, 1).is_err());
}
#[test]
fn route_and_model_change_fences_only_the_owned_workspace() {
    let (mut core, _) = setup(memory());
    let a = workspace(&core);
    let b = workspace(&core);
    let ar = uuid::Uuid::new_v4().to_string();
    let br = uuid::Uuid::new_v4().to_string();
    core.start(&a, "a", &ar).unwrap();
    core.start(&b, "b", &br).unwrap();
    let a_request = core.store.runtime_request(&ar).unwrap().unwrap();
    let b_request = core.store.runtime_request(&br).unwrap().unwrap();
    let mut a_model = core.store.runtime_workspace(&a).unwrap();
    a_model.generation += 1;
    a_model.model_id = "changed".into();
    core.store.runtime_save_workspace(&a_model).unwrap();
    assert!(core.store.runtime_current(&a_request).is_err());
    assert!(core.store.runtime_current(&b_request).is_ok());
    assert!(core
        .store
        .runtime_finish(&a_request, "complete", "stale")
        .is_err());
    core.store
        .runtime_finish(&b_request, "complete", "current")
        .unwrap();
}
#[test]
fn cancellation_and_deleted_workspaces_reject_late_publication() {
    let (mut core, _) = setup(memory());
    let id = workspace(&core);
    let request = uuid::Uuid::new_v4().to_string();
    core.start(&id, "pending", &request).unwrap();
    let mut captured = core.store.runtime_request(&request).unwrap().unwrap();
    captured.cancel_requested = true;
    core.store.runtime_save_request(&captured).unwrap();
    assert!(core
        .store
        .runtime_finish(&captured, "complete", "late")
        .is_err());
    core.store.delete(&id).unwrap();
    assert!(core.store.runtime_current(&captured).is_err());
    assert!(core
        .store
        .runtime_finish(&captured, "complete", "resurrect")
        .is_err());
    assert!(core.store.summaries().unwrap().is_empty());
}
#[test]
fn public_runtime_status_has_no_manual_connection_or_credentials() {
    let (core, secrets) = setup(memory());
    let value = serde_json::to_value(core.store.runtime_status().unwrap()).unwrap();
    for field in [
        "endpoint",
        "enabled",
        "configured",
        "allowTailscaleHttp",
        "hasToken",
        "authToken",
        "configEpoch",
    ] {
        assert!(value.get(field).is_none(), "{field}");
    }
    assert_eq!(value["state"], "ready");
    assert!(
        secrets.reads.lock().unwrap().is_empty(),
        "status never reads keys"
    );
    assert!(serde_json::from_value::<Route>(json!("direct")).is_err());
}
#[tokio::test]
async fn quit_does_not_wait_for_the_keychain_or_core_mutex() {
    let (core, _) = setup(memory());
    let state = NativeState::new(Ok(core));
    let _blocked_keychain_guard = state.lock().unwrap();
    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        managed::shutdown(&state),
    )
    .await
    .unwrap();
    assert!(state
        .runtime_stopping
        .load(std::sync::atomic::Ordering::Acquire));
}
#[test]
fn model_failure_keeps_local_control_and_pause_but_blocks_new_execution() {
    let (core, _) = setup(memory());
    crate::core::model_check_failed(&core).unwrap();
    let status = core.store.runtime_status().unwrap();
    assert!(status.verified);
    assert_eq!(status.state, "error");
    assert!(require_binding(&status, false).is_ok());
    assert!(require_binding(&status, true).is_err());
    let pause = resources::Resource::PauseSchedule {
        id: uuid::Uuid::new_v4().to_string(),
        expected_version: 1,
    };
    assert!(resources::authorize_resource(&status, &pause).is_ok());
}
#[test]
fn provider_edit_and_cleared_cache_cannot_replace_owned_profile_identity() {
    let (mut core, _) = setup(memory());
    let status = core.store.runtime_status().unwrap();
    let mut caps = status.capabilities.unwrap();
    managed::accept_snapshot(&mut core, &caps, &status.models, None).unwrap();
    let library = caps.library_id.clone();
    core.configure(ProviderInput {
        label: "Changed".into(),
        base_url: "https://new-model.example".into(),
        model: "new-model".into(),
        api_key: None,
        clear_key: false,
    })
    .unwrap();
    assert!(
        core.store.runtime_status().unwrap().verified,
        "provider edits do not erase control authentication"
    );
    core.store
        .db
        .execute("UPDATE runtime_config SET capabilities=NULL WHERE id=1", [])
        .unwrap();
    caps.library_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        managed::accept_snapshot(&mut core, &caps, &status.models, None)
            .err()
            .unwrap()
            .code,
        "runtime_binding_changed"
    );
    assert_eq!(
        core.store
            .runtime_config()
            .unwrap()
            .bound_library_id
            .as_deref(),
        Some(library.as_str())
    );
}
#[tokio::test]
async fn stale_catalog_cannot_borrow_rotated_provider_credentials_after_waiting_for_gate() {
    let (core, secrets) = setup(memory());
    let state = NativeState::new(Ok(core));
    let gate = state.runtime_gate.lock().await;
    {
        let mut core = state.lock().unwrap();
        core.configure(ProviderInput {
            label: "Rotated".into(),
            base_url: "https://models.example".into(),
            model: "new-model".into(),
            api_key: None,
            clear_key: false,
        })
        .unwrap();
        let mut provider = core.store.provider().unwrap();
        provider.config.verified = true;
        core.store.set_provider(&provider).unwrap();
    }
    drop(gate);
    assert_eq!(
        managed::sync_model(&state, Some((0, vec!["old-catalog-model".into()])))
            .await
            .unwrap_err()
            .code,
        "stale_request"
    );
    assert!(secrets.reads.lock().unwrap().is_empty());
    assert!(state
        .lock()
        .unwrap()
        .store
        .runtime_config()
        .unwrap()
        .approved_models
        .is_empty());
}
#[test]
fn explicit_quit_interrupts_pending_without_forging_a_cancel_receipt_or_losing_drafts() {
    let (mut core, _) = setup(memory());
    let id = workspace(&core);
    let request = uuid::Uuid::new_v4().to_string();
    core.start(&id, "pending user text", &request).unwrap();
    core.store.update_draft(&id, "next exact draft", 3).unwrap();
    let result = runs::interrupt_for_quit(&core, &id, &request).unwrap();
    assert_eq!(result.draft, "next exact draft");
    assert_eq!(result.messages[0].content, "pending user text");
    assert_eq!(result.messages[1].status, "error");
    assert!(result.messages[1].content.contains("interrupted"));
    assert!(!result.messages[1].content.contains("cancelled"));
    assert_eq!(
        core.store
            .runtime_request(&request)
            .unwrap()
            .unwrap()
            .progress
            .state,
        "interrupted"
    );
    assert!(runs::interrupt_for_quit(&core, &id, &uuid::Uuid::new_v4().to_string()).is_err());
}
#[test]
fn provider_rotation_fences_hermes_message_rows_and_keeps_text() {
    let (mut core, _) = setup(memory());
    let id = workspace(&core);
    let request = uuid::Uuid::new_v4().to_string();
    core.start(&id, "pending", &request).unwrap();
    core.configure(ProviderInput {
        label: "Direct".into(),
        base_url: "https://another-direct.example".into(),
        model: "direct-two".into(),
        api_key: None,
        clear_key: false,
    })
    .unwrap();
    assert_eq!(
        core.store.workspace(&id).unwrap().messages[1].status,
        "cancelled"
    );
    assert!(core
        .store
        .runtime_current(&core.store.runtime_request(&request).unwrap().unwrap())
        .is_err());
}
#[test]
fn known_remote_workspace_never_reimports_local_history() {
    let (mut core, _) = setup(memory());
    let id = workspace(&core);
    let mut ws = core.store.runtime_workspace(&id).unwrap();
    ws.remote_initialized = true;
    core.store.runtime_save_workspace(&ws).unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    core.start(&id, "next", &request).unwrap();
    let captured = core.store.runtime_request(&request).unwrap().unwrap();
    assert!(captured.input.import_history.is_empty());
    assert!(serde_json::to_value(captured.input)
        .unwrap()
        .get("importHistory")
        .is_none());
}
#[test]
fn events_match_actual_tagged_protocol_and_reject_unknown_kinds() {
    let event = json!({"runId":"11111111-1111-4111-8111-111111111111","seq":1,"at":"2026-09-10T12:00:00Z","type":"assistant.delta","payload":{"text":"provisional"}});
    let parsed: Event = serde_json::from_value(event.clone()).unwrap();
    assert!(matches!(parsed.kind, EventKind::Delta { .. }));
    assert_eq!(serde_json::to_value(parsed).unwrap(), event);
    let mut bad = event;
    bad["type"] = json!("native.execute");
    assert!(serde_json::from_value::<Event>(bad).is_err());
}
#[test]
fn provider_key_rotation_preserves_managed_history_identity() {
    let (mut core, _) = setup(memory());
    let checked = core.store.runtime_status().unwrap();
    let id = workspace(&core);
    let mut ws = core.store.runtime_workspace(&id).unwrap();
    ws.remote_initialized = true;
    core.store.runtime_save_workspace(&ws).unwrap();
    let epoch = core.store.runtime_config().unwrap().config_epoch;
    core.configure(ProviderInput {
        label: "Rotated".into(),
        base_url: "https://models.example".into(),
        model: "direct-model".into(),
        api_key: Some("rotated-model-key".into()),
        clear_key: false,
    })
    .unwrap();
    assert_ne!(core.store.runtime_config().unwrap().config_epoch, epoch);
    assert!(!core.store.runtime_config().unwrap().background_approved);
    let generation = core.store.provider().unwrap().generation;
    managed::accept_snapshot(
        &mut core,
        checked.capabilities.as_ref().unwrap(),
        &checked.models,
        Some(generation),
    )
    .unwrap();
    assert!(
        core.store
            .runtime_workspace(&id)
            .unwrap()
            .remote_initialized
    );
}
fn poll(
    request: &storage::StoredRequest,
    state: RunState,
    seq: u64,
    last_seq: u64,
    has_more: bool,
) -> RunPoll {
    RunPoll {
        run: Run {
            id: request.progress.request_id.clone(),
            workspace_id: request.input.workspace_id.clone(),
            origin: "operator".into(),
            schedule_id: None,
            state,
            reason: None,
            model_id: request.input.model_id.clone(),
            hermes_revision: "349e6611a1c5d846a865368dd6c386b78edd1a54".into(),
            budgets: Budgets::default(),
            created_at: now(),
            started_at: None,
            finished_at: None,
            last_seq,
            final_message: Some("final, not provisional".into()),
        },
        events: vec![Event {
            run_id: request.progress.request_id.clone(),
            seq,
            at: now(),
            kind: EventKind::Delta {
                text: "provisional".into(),
            },
        }],
        next_after: seq,
        has_more,
        tool_requests: vec![],
    }
}
#[test]
fn event_cursor_commits_before_delivery_and_replayed_or_gapped_pages_never_publish() {
    let (mut core, _) = setup(memory());
    let id = workspace(&core);
    let request = uuid::Uuid::new_v4().to_string();
    core.start(&id, "cursor", &request).unwrap();
    let captured = core.store.runtime_request(&request).unwrap().unwrap();
    let (_, _, complete) = runs::apply_poll(
        &mut core,
        &request,
        poll(&captured, RunState::Succeeded, 1, 2, true),
    )
    .unwrap();
    assert!(!complete);
    assert_eq!(
        core.store.workspace(&id).unwrap().messages[1].status,
        "pending"
    );
    assert_eq!(
        core.store
            .runtime_request(&request)
            .unwrap()
            .unwrap()
            .progress
            .cursor,
        1
    );
    assert!(runs::apply_poll(
        &mut core,
        &request,
        poll(&captured, RunState::Succeeded, 1, 2, true)
    )
    .is_err());
    assert!(runs::apply_poll(
        &mut core,
        &request,
        poll(&captured, RunState::Succeeded, 3, 3, false)
    )
    .is_err());
    let (_, _, complete) = runs::apply_poll(
        &mut core,
        &request,
        poll(&captured, RunState::Succeeded, 2, 2, false),
    )
    .unwrap();
    assert!(complete);
    assert_eq!(
        core.store.workspace(&id).unwrap().messages[1].content,
        "final, not provisional"
    );
    assert!(runs::apply_poll(
        &mut core,
        &request,
        poll(&captured, RunState::Succeeded, 2, 2, false)
    )
    .is_err());
}
#[test]
fn wrong_model_workspace_and_request_events_never_enter_history() {
    let (mut core, _) = setup(memory());
    let id = workspace(&core);
    let request = uuid::Uuid::new_v4().to_string();
    core.start(&id, "bound", &request).unwrap();
    let captured = core.store.runtime_request(&request).unwrap().unwrap();
    let mut wrong = poll(&captured, RunState::Succeeded, 1, 1, false);
    wrong.run.model_id = "substituted".into();
    assert!(runs::apply_poll(&mut core, &request, wrong).is_err());
    let mut wrong = poll(&captured, RunState::Succeeded, 1, 1, false);
    wrong.run.workspace_id = uuid::Uuid::new_v4().to_string();
    assert!(runs::apply_poll(&mut core, &request, wrong).is_err());
    let mut wrong = poll(&captured, RunState::Succeeded, 1, 1, false);
    wrong.events[0].run_id = uuid::Uuid::new_v4().to_string();
    assert!(runs::apply_poll(&mut core, &request, wrong).is_err());
    assert_eq!(
        core.store.workspace(&id).unwrap().messages[1].status,
        "pending"
    );
    assert_eq!(
        core.store
            .runtime_request(&request)
            .unwrap()
            .unwrap()
            .progress
            .cursor,
        0
    );
}
#[test]
fn preadmission_cancel_receipt_is_not_a_fabricated_completed_run() {
    let receipt: CancelReceipt = serde_json::from_value(
        json!({"id":uuid::Uuid::new_v4().to_string(),"state":"cancelled","admitted":false}),
    )
    .unwrap();
    assert!(!receipt.admitted);
    assert!(receipt.run.is_none());
    assert_eq!(receipt.state, RunState::Cancelled);
}
#[test]
fn checked_binding_is_independent_from_worker_readiness() {
    let (mut core, _) = setup(memory());
    let status = core.store.runtime_status().unwrap();
    let mut caps = status.capabilities.unwrap();
    caps.runtime.ready = false;
    let checked = managed::accept_snapshot(&mut core, &caps, &status.models, Some(0)).unwrap();
    assert!(checked.config.verified);
    assert!(
        session(&core, false).is_ok(),
        "authenticated stop transport remains available"
    );
    assert!(session(&core, true).is_err());
    for operation in ["pauseSchedule", "listSchedules"] {
        let input = if operation == "pauseSchedule" {
            json!({"operation":operation,"id":uuid::Uuid::new_v4().to_string(),"expectedVersion":1})
        } else {
            json!({"operation":operation,"workspaceId":uuid::Uuid::new_v4().to_string()})
        };
        let resource = serde_json::from_value(input).unwrap();
        assert!(resources::authorize_resource(&checked, &resource).is_ok());
    }
    let enable = resources::Resource::EnableSchedule {
        id: uuid::Uuid::new_v4().to_string(),
        expected_version: 1,
        consent: ScheduleConsent {
            schedule_version: 1,
            model_id: "runtime-model".into(),
            grant_refs: vec![],
        },
    };
    assert!(resources::authorize_resource(&checked, &enable).is_err());
}

#[test]
fn changed_identity_never_reuses_old_capabilities_to_authorize_mutation() {
    for field in ["modelOrigin", "libraryId", "deviceId"] {
        let (mut core, _) = setup(memory());
        let status = core.store.runtime_status().unwrap();
        let mut config = core.store.runtime_config().unwrap();
        config.config.verified = false;
        core.store
            .db
            .execute(
                "UPDATE runtime_config SET value=?1 WHERE id=1",
                [storage::encode(&config).unwrap()],
            )
            .unwrap();
        let mut caps = serde_json::to_value(status.capabilities.unwrap()).unwrap();
        caps[field] = if field == "modelOrigin" {
            json!("https://substituted.example")
        } else {
            json!(uuid::Uuid::new_v4().to_string())
        };
        let caps = serde_json::from_value(caps).unwrap();
        assert_eq!(
            managed::accept_snapshot(&mut core, &caps, &status.models, Some(0))
                .err()
                .unwrap()
                .code,
            "runtime_binding_changed"
        );
        let current = core.store.runtime_status().unwrap();
        assert!(!current.config.verified);
        assert!(
            current.capabilities.is_some(),
            "cached identity is not verification"
        );
        let resource = resources::Resource::EnableSchedule {
            id: uuid::Uuid::new_v4().to_string(),
            expected_version: 1,
            consent: ScheduleConsent {
                schedule_version: 1,
                model_id: "runtime-model".into(),
                grant_refs: vec![],
            },
        };
        assert!(resources::authorize_resource(&current, &resource).is_err());
        assert!(session(&core, true).is_err());
    }
}

#[test]
fn resource_only_remote_intent_survives_lost_response_and_restart_without_initializing_history() {
    let temp = tempfile::tempdir().unwrap();
    let (core, _) = setup(Store::open(temp.path()).unwrap());
    let id = workspace(&core);
    let generation = core.store.runtime_config().unwrap().config.generation;
    core.store.runtime_own_workspace(&id, generation).unwrap();
    // No response or run admission is recorded: schedule/proposal creation may have reached the server.
    assert!(
        !core
            .store
            .runtime_workspace(&id)
            .unwrap()
            .remote_initialized
    );
    assert!(!core.store.runtime_owners(&id).unwrap().is_empty());
    drop(core);
    let store = Store::open(temp.path()).unwrap();
    assert!(!store.runtime_workspace(&id).unwrap().remote_initialized);
    let owner = store.runtime_deletion_owner(&id).unwrap().unwrap();
    assert!(store
        .runtime_acknowledge_deletion(&id, &owner, generation + 1)
        .is_err());
    assert_eq!(store.runtime_owners(&id).unwrap().len(), 1);
    store
        .runtime_acknowledge_deletion(&id, &owner, generation)
        .unwrap();
    assert!(store.runtime_deletion_owner(&id).unwrap().is_none());
}

#[test]
fn legacy_remote_migration_archives_intents_without_replay_and_keeps_chats() {
    let (mut core, _) = setup(memory());
    let id = workspace(&core);
    core.store.update_draft(&id, "keep draft", 1).unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    core.start(&id, "old remote question", &request).unwrap();
    core.store.update_draft(&id, "next draft", 3).unwrap();
    core.store.db.execute("INSERT INTO runtime_owners VALUES(?1,'https://old-remote.example','old-library','old-device')", [&id]).unwrap();
    core.store.db.execute("UPDATE runtime_workspaces SET value=json_set(value,'$.route','direct') WHERE workspace_id=?1", [&id]).unwrap();
    core.store.db.execute_batch("DROP TABLE runtime_legacy_requests; DROP TABLE runtime_legacy_owners; PRAGMA user_version=4;").unwrap();
    storage::migrate(&mut core.store.db).unwrap();
    assert_eq!(
        core.store
            .db
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        5
    );
    assert!(core.store.runtime_request(&request).unwrap().is_none());
    assert!(core.store.runtime_owners(&id).unwrap().is_empty());
    assert_eq!(
        core.store
            .db
            .query_row("SELECT count(*) FROM runtime_legacy_owners", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        core.store
            .db
            .query_row("SELECT count(*) FROM runtime_legacy_requests", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        1
    );
    let chat = core.store.workspace(&id).unwrap();
    assert_eq!(chat.messages[0].content, "old remote question");
    assert_eq!(chat.messages[1].status, "error");
    assert!(chat.messages[1]
        .content
        .contains("not cancelled or replayed"));
    assert_eq!(chat.draft, "next draft");
    let workspace = core.store.runtime_workspace(&id).unwrap();
    assert_eq!(workspace.route, Route::Hermes);
    assert_eq!(workspace.model_id, "runtime-model");
    assert!(!workspace.remote_initialized);
}
#[test]
fn recovered_cancel_resumes_same_intent_and_admitted_receipt_initializes_history_before_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let (mut core, secrets) = setup(Store::open(temp.path()).unwrap());
    let id = workspace(&core);
    let request_id = uuid::Uuid::new_v4().to_string();
    core.start(&id, "first admission response lost", &request_id)
        .unwrap();
    let before_cancel = core.store.runtime_request(&request_id).unwrap().unwrap();
    let mut request = before_cancel.clone();
    request.dispatched = true;
    request.cancel_requested = true;
    core.store.runtime_save_request(&request).unwrap();
    assert!(
        core.store.runtime_current(&before_cancel).is_err(),
        "stale in-flight copies cannot replay after durable Stop"
    );
    drop(core);
    let mut core = Core::new(Store::open(temp.path()).unwrap(), Box::new(secrets));
    core.controller = Some(transport::Controller::fixture());
    let mut recovered = core.store.runtime_request(&request_id).unwrap().unwrap();
    assert!(recovered.cancel_requested && recovered.dispatched);
    assert!(
        core.store.runtime_pending(&recovered).is_ok(),
        "complete can resume cancellation after restart"
    );
    assert!(
        core.store.runtime_current(&recovered).is_err(),
        "normal admission must not resume"
    );
    let run = poll(&recovered, RunState::Cancelling, 1, 1, false).run;
    runs::apply_cancel_receipt(
        &core,
        &mut recovered,
        CancelReceipt {
            id: request_id.clone(),
            state: RunState::Cancelling,
            admitted: true,
            run: Some(run),
        },
    )
    .unwrap();
    assert!(
        core.store
            .runtime_workspace(&id)
            .unwrap()
            .remote_initialized
    );
    assert!(core.store.runtime_finish_cancel(&recovered).is_err());
    assert_eq!(
        core.store.workspace(&id).unwrap().messages[1].status,
        "pending"
    );
    assert!(core
        .start(&id, "unsafe duplicate", &uuid::Uuid::new_v4().to_string())
        .is_err());
    let mut run = recovered.progress.run.clone().unwrap();
    run.state = RunState::Cancelled;
    runs::apply_cancel_receipt(
        &core,
        &mut recovered,
        CancelReceipt {
            id: request_id.clone(),
            state: RunState::Cancelled,
            admitted: true,
            run: Some(run),
        },
    )
    .unwrap();
    core.store.runtime_finish_cancel(&recovered).unwrap();
    assert_eq!(
        core.store.workspace(&id).unwrap().messages[1].status,
        "cancelled"
    );
    let next = uuid::Uuid::new_v4().to_string();
    core.start(&id, "next admitted turn", &next).unwrap();
    assert!(core
        .store
        .runtime_request(&next)
        .unwrap()
        .unwrap()
        .input
        .import_history
        .is_empty());
}

#[test]
fn stop_racing_with_completion_returns_durable_terminal_snapshot() {
    let (mut core, _) = setup(memory());
    let id = workspace(&core);
    let request_id = uuid::Uuid::new_v4().to_string();
    core.start(&id, "finish before Stop acquires lock", &request_id)
        .unwrap();
    assert!(runs::terminal_workspace(&core, &id, &request_id)
        .unwrap()
        .is_none());
    let request = core.store.runtime_request(&request_id).unwrap().unwrap();
    runs::apply_poll(
        &mut core,
        &request_id,
        poll(&request, RunState::Succeeded, 1, 1, false),
    )
    .unwrap();
    let terminal = runs::terminal_workspace(&core, &id, &request_id)
        .unwrap()
        .unwrap();
    assert_eq!(terminal.messages[1].status, "complete");
    assert_eq!(terminal.messages[1].content, "final, not provisional");
}

#[test]
fn invalid_cancel_receipts_do_not_initialize_history_and_tombstone_finishes_without_admission() {
    let (mut core, _) = setup(memory());
    let id = workspace(&core);
    let request_id = uuid::Uuid::new_v4().to_string();
    core.start(&id, "uncertain", &request_id).unwrap();
    let mut request = core.store.runtime_request(&request_id).unwrap().unwrap();
    request.cancel_requested = true;
    request.dispatched = true;
    core.store.runtime_save_request(&request).unwrap();
    let mut run = poll(&request, RunState::Cancelled, 1, 1, false).run;
    run.model_id = "wrong-model".into();
    assert!(runs::apply_cancel_receipt(
        &core,
        &mut request,
        CancelReceipt {
            id: request_id.clone(),
            state: RunState::Cancelled,
            admitted: true,
            run: Some(run)
        }
    )
    .is_err());
    assert!(
        !core
            .store
            .runtime_workspace(&id)
            .unwrap()
            .remote_initialized
    );
    assert_eq!(
        core.store.workspace(&id).unwrap().messages[1].status,
        "pending"
    );
    runs::apply_cancel_receipt(
        &core,
        &mut request,
        CancelReceipt {
            id: request_id.clone(),
            state: RunState::Cancelled,
            admitted: false,
            run: None,
        },
    )
    .unwrap();
    core.store.runtime_finish_cancel(&request).unwrap();
    assert!(
        !core
            .store
            .runtime_workspace(&id)
            .unwrap()
            .remote_initialized
    );
    assert_eq!(
        core.store.workspace(&id).unwrap().messages[1].status,
        "cancelled"
    );
}

#[test]
fn import_history_rejects_oversized_entries_and_aggregate_before_consuming_draft() {
    for oversized in [
        vec!["x".repeat(16_001)],
        vec!["🦀".repeat(16_001)],
        vec!["🦀".repeat(16_000); 3],
    ] {
        let (mut core, _) = setup(memory());
        let id = workspace(&core);
        for content in oversized {
            let previous = uuid::Uuid::new_v4().to_string();
            core.store
                .start(&id, "previous user", &previous, 0)
                .unwrap();
            core.store
                .finish(&id, &previous, 0, "complete", &content)
                .unwrap();
        }
        core.store
            .update_draft(&id, "keep exact draft", 10)
            .unwrap();
        let before = serde_json::to_value(core.store.workspace(&id).unwrap()).unwrap();
        let request = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            core.start(&id, "keep exact draft", &request)
                .err()
                .unwrap()
                .code,
            "context_limit"
        );
        assert_eq!(
            serde_json::to_value(core.store.workspace(&id).unwrap()).unwrap(),
            before
        );
        assert!(core.store.runtime_request(&request).unwrap().is_none());
    }
    let (mut core, _) = setup(memory());
    let id = workspace(&core);
    let prior = uuid::Uuid::new_v4().to_string();
    core.store.start(&id, "previous", &prior, 0).unwrap();
    let unicode = "🦀".repeat(16_000);
    core.store
        .finish(&id, &prior, 0, "complete", &unicode)
        .unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    core.start(&id, "current", &request).unwrap();
    assert_eq!(
        core.store
            .runtime_request(&request)
            .unwrap()
            .unwrap()
            .input
            .import_history[1]
            .content,
        unicode,
        "Unicode characters, not bytes, and no truncation"
    );
}

#[test]
fn actual_miniforum_synthetic_application_responses_match_native_contracts() {
    use serde_json::Value;
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Page<T> {
        items: Vec<T>,
        has_more: bool,
        next_cursor: Option<String>,
    }
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../runtime/tests/fixtures/protocol-responses.json"
    ))
    .unwrap();
    let caps: Capabilities = serde_json::from_value(fixture["capabilities"].clone()).unwrap();
    let models: Items<RuntimeModel> = serde_json::from_value(fixture["models"].clone()).unwrap();
    validate_capabilities(&caps, &models.items).unwrap();
    let _: Run = serde_json::from_value(fixture["runAccepted"].clone()).unwrap();
    let _: RunPoll = serde_json::from_value(fixture["runPoll"].clone()).unwrap();
    for key in ["cancelKnown", "cancelMissing"] {
        let _: CancelReceipt = serde_json::from_value(fixture[key].clone()).unwrap();
    }
    let interface: Interface = serde_json::from_value(fixture["interfaceDetail"].clone()).unwrap();
    interfaces::validate(&interface.spec).unwrap();
    let proposal: Proposal = serde_json::from_value(fixture["proposalDetail"].clone()).unwrap();
    interfaces::validate(&proposal.spec).unwrap();
    let revision: InterfaceRevision =
        serde_json::from_value(fixture["revisionDetail"].clone()).unwrap();
    interfaces::validate(&revision.spec).unwrap();
    let _: Schedule = serde_json::from_value(fixture["scheduleDetail"].clone()).unwrap();
    let page: Page<Interface> = serde_json::from_value(fixture["interfacesPage"].clone()).unwrap();
    assert!(!page.has_more);
    assert!(page.next_cursor.is_none());
    assert!(page.items.iter().all(|i| i.spec.is_null()));
    let _: Page<Proposal> = serde_json::from_value(fixture["proposalsPage"].clone()).unwrap();
    let _: Page<InterfaceRevision> =
        serde_json::from_value(fixture["revisionsPage"].clone()).unwrap();
    let _: Page<Schedule> = serde_json::from_value(fixture["schedulesPage"].clone()).unwrap();
    let _: Page<Run> = serde_json::from_value(fixture["runsPage"].clone()).unwrap();
    let input = ProposalInput {
        workspace_id: uuid::Uuid::new_v4().to_string(),
        interface_id: None,
        expected_revision: 0,
        title: "New".into(),
        spec: interface.spec,
    };
    assert!(serde_json::to_value(input)
        .unwrap()
        .get("interfaceId")
        .is_none());
}
