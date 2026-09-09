// Included only inside core::tests. Run explicitly against the synthetic fixture
// on the designated test host, optionally through an SSH tunnel. Never starts a local backend.
#[tokio::test]
#[ignore = "requires explicit FORMA_TEST_PROVIDER_URL pointing to the synthetic fixture"]
async fn external_fixture_real_http_lifecycle() {
    use std::{sync::Arc, time::Duration};

    let base = std::env::var("FORMA_TEST_PROVIDER_URL")
        .expect("Set FORMA_TEST_PROVIDER_URL to the isolated synthetic fixture URL");
    let base = validation::endpoint(&base).expect("Valid fixture endpoint required");
    assert!(!base.is_empty());
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    // Test-only memory credentials. This does NOT exercise the OS keyring or app UI.
    let state = Arc::new(NativeState::new(Ok(Core::new(
        store,
        Box::<TestCredentials>::default(),
    ))));
    state
        .lock()
        .unwrap()
        .configure(ProviderInput {
            label: "Synthetic external fixture".into(),
            base_url: base.clone(),
            model: "forma-fixture".into(),
            api_key: None,
            clear_key: false,
        })
        .unwrap();
    let checked = check(&state).await.unwrap();
    assert!(checked.provider.verified);
    assert_eq!(checked.models, vec!["forma-fixture"]);
    assert!(!checked.provider.has_key);

    let workspace = state
        .lock()
        .unwrap()
        .store
        .create_workspace()
        .unwrap()
        .summary
        .id;
    state
        .lock()
        .unwrap()
        .store
        .update_draft(&workspace, "forma-fixture:success", 1)
        .unwrap();
    let request = uuid::Uuid::new_v4().to_string();
    let started = state
        .lock()
        .unwrap()
        .start(&workspace, "forma-fixture:success", &request)
        .unwrap();
    assert_eq!(started.messages.len(), 2);
    assert_eq!(started.messages[1].status, "pending");
    assert!(started.draft.is_empty());
    let completed = complete(&state, workspace.clone(), request.clone())
        .await
        .unwrap();
    assert_eq!(
        completed.messages[1].content,
        "Synthetic Forma fixture reply."
    );
    assert_eq!(completed.messages[1].status, "complete");
    assert!(complete(&state, workspace.clone(), request.clone())
        .await
        .is_err());
    assert!(state
        .lock()
        .unwrap()
        .start(&workspace, "forma-fixture:success", &request)
        .is_err());

    for mode in ["error", "malformed"] {
        let request = uuid::Uuid::new_v4().to_string();
        state
            .lock()
            .unwrap()
            .start(&workspace, &format!("forma-fixture:{mode}"), &request)
            .unwrap();
        let completed = complete(&state, workspace.clone(), request).await.unwrap();
        let last = completed.messages.last().unwrap();
        assert_eq!(last.status, "error");
        assert!(!last.content.contains("fixture-only-diagnostic"));
    }

    // Hold a real HTTP response open, then cancel it. Repeat for workspace deletion
    // and a provider generation change; neither may apply a late completion.
    for mode in ["cancel", "delete", "config"] {
        let id = state
            .lock()
            .unwrap()
            .store
            .create_workspace()
            .unwrap()
            .summary
            .id;
        let request = uuid::Uuid::new_v4().to_string();
        state
            .lock()
            .unwrap()
            .start(&id, &format!("forma-fixture:slow-{mode}"), &request)
            .unwrap();
        let task_state = state.clone();
        let task_id = id.clone();
        let task_request = request.clone();
        let task = tokio::spawn(async move { complete(&task_state, task_id, task_request).await });
        // Wait for the fixture to receipt the actual POST, rather than assuming that
        // starting the async task means a packet reached the server.
        let receipt_url = format!("{base}/receipt");
        let expected = format!("forma-fixture:slow-{mode}");
        let mut reached = false;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let receipt: serde_json::Value = state
                .client
                .as_ref()
                .unwrap()
                .get(&receipt_url)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if receipt["requests"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["lastUser"] == expected)
            {
                reached = true;
                break;
            }
        }
        assert!(reached, "Fixture did not receive slow request");
        assert!(complete(&state, id.clone(), request.clone()).await.is_err());
        match mode {
            "cancel" => {
                state.lock().unwrap().cancel(&id, &request).unwrap();
            }
            "delete" => {
                state.lock().unwrap().delete(&id).unwrap();
            }
            _ => {
                state
                    .lock()
                    .unwrap()
                    .configure(ProviderInput {
                        label: "Synthetic external fixture".into(),
                        base_url: base.clone(),
                        model: "forma-fixture".into(),
                        api_key: None,
                        clear_key: false,
                    })
                    .unwrap();
            }
        }
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("Cancellation must interrupt the real HTTP future promptly")
            .unwrap();
        assert!(result.is_err());
        if mode == "delete" {
            assert!(state.lock().unwrap().store.workspace(&id).is_err());
            assert!(state
                .lock()
                .unwrap()
                .store
                .update_draft(&id, "late", 9)
                .is_err());
        } else {
            assert_eq!(
                state.lock().unwrap().store.workspace(&id).unwrap().messages[1].status,
                "cancelled"
            );
        }
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let receipt: serde_json::Value = state
        .client
        .as_ref()
        .unwrap()
        .get(format!("{base}/receipt"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let requests = receipt["requests"].as_array().unwrap();
    let posts: Vec<_> = requests
        .iter()
        .filter(|event| event["method"] == "POST")
        .collect();
    assert_eq!(
        posts.len(),
        6,
        "No hidden retry, duplicate complete, or automatic resend is allowed"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|event| event["method"] == "GET")
            .count(),
        1
    );
    println!(
        "EXTERNAL_FIXTURE_RECEIPT {}",
        serde_json::to_string(&receipt).unwrap()
    );
    println!("PASS: real HTTP models, durable start/complete, no repeat, HTTP error/malformed handling, cancellation, deletion, and config invalidation. OS keyring and native UI NOT TESTED.");
    drop(state);
    let reopened = Store::open(directory.path()).unwrap();
    assert_eq!(
        reopened.workspace(&workspace).unwrap().messages[1].content,
        "Synthetic Forma fixture reply."
    );
}
