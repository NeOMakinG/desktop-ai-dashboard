use super::*;
use crate::connectors::keychain::tests::MemorySecrets;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};

type Hook = Box<dyn FnOnce() + Send>;
#[derive(Default)]
struct FixtureTransport {
    responses: Mutex<VecDeque<AppResult<serde_json::Value>>>,
    requests: Mutex<Vec<Request>>,
    hook: Mutex<Option<Hook>>,
    calls: AtomicUsize,
}
impl FixtureTransport {
    fn new(responses: Vec<serde_json::Value>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().map(Ok).collect()),
            ..Self::default()
        }
    }
}
impl Transport for FixtureTransport {
    fn send(
        &self,
        request: Request,
    ) -> Pin<Box<dyn Future<Output = AppResult<Zeroizing<Vec<u8>>>> + Send + '_>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().unwrap().push(request);
            let hook = self.hook.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
            let result = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected fixture request")?;
            Ok(Zeroizing::new(serde_json::to_vec(&result).unwrap()))
        })
    }
}
fn at(seconds: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::seconds(seconds))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
fn setup(
    operation: ReadOperation,
) -> (Arc<ConnectorsCore>, RuntimeReadContext, Arc<MemorySecrets>) {
    let secrets = Arc::new(MemorySecrets::default());
    let core = Arc::new(ConnectorsCore {
        lifecycle: Mutex::new(Lifecycle::default()),
        store: Mutex::new(store::tests::memory()),
        secrets: secrets.clone(),
        client: build_client().unwrap(),
    });
    let status = ConnectorStatus {
        id: uuid::Uuid::new_v4().to_string(),
        provider: "google".into(),
        scopes: DEFAULT_SCOPES.iter().map(|s| (*s).into()).collect(),
        display_name: None,
        connected_at: at(-60),
        expires_at: at(3600),
    };
    store_guard(&core).unwrap().upsert(&status).unwrap();
    let payload = serialize_tokens(
        "fixture-access-secret",
        Some("fixture-refresh-secret"),
        &status.expires_at,
    )
    .unwrap();
    secrets.write(&status.id, &payload).unwrap();
    let grant = lifecycle_guard(&core)
        .unwrap()
        .grants
        .register(
            GrantRequest {
                workspace_id: "workspace-1".into(),
                device_id: "mac-1".into(),
                connection_id: status.id.clone(),
                operations: vec![operation],
                expires_at: at(600),
                account_read_consented: true,
                egress: EgressConsent {
                    runtime_origin: "https://runtime.example.test".into(),
                    model_origin: "https://model.example.test".into(),
                    data_categories: vec![operation.category().into()],
                    consented: true,
                    retention: "ephemeral-run".into(),
                },
            },
            &status,
            chrono::Utc::now().timestamp_millis(),
        )
        .unwrap();
    let context = RuntimeReadContext {
        workspace_id: grant.scope.workspace_id,
        device_id: grant.scope.device_id,
        connection_id: status.id,
        grant_id: grant.grant_id,
        generation: grant.generation,
        run_id: "run-1".into(),
        runtime_origin: grant.scope.egress.runtime_origin,
        model_origin: grant.scope.egress.model_origin,
        operation,
        args: json!({"startAt":at(-86400), "endAt":at(0), "maxItems":100}),
        expires_at: at(90),
    };
    (core, context, secrets)
}
fn message(id: &str, age: i64) -> serde_json::Value {
    json!({"id":id,"internalDate":(chrono::Utc::now().timestamp_millis() + age * 1000).to_string(),
        "labelIds":["INBOX","UNREAD"],"payload":{"headers":[{"name":"From","value":"Sender <sender@example.test>"},{"name":"Subject","value":"Synthetic café"}],
        "body":{"data":"BODY_MUST_NOT_LEAVE"},"parts":[{"filename":"SECRET_ATTACHMENT"}]},
        "snippet":"SNIPPET_MUST_NOT_LEAVE","access_token":"TOKEN_FIELD_MUST_NOT_LEAVE"})
}
fn pairs(url: &reqwest::Url) -> Vec<(String, String)> {
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}
fn safe_result(result: &ConnectorReadResult) -> serde_json::Value {
    let value = serde_json::to_value(result).unwrap();
    let serialized = value.to_string();
    for secret in [
        "fixture-access-secret",
        "fixture-refresh-secret",
        "BODY_MUST_NOT_LEAVE",
        "SECRET_ATTACHMENT",
        "SNIPPET_MUST_NOT_LEAVE",
        "TOKEN_FIELD_MUST_NOT_LEAVE",
    ] {
        assert!(
            !serialized.contains(secret),
            "sensitive fixture marker in result"
        );
    }
    value
}

#[tokio::test]
async fn gmail_exact_inbox_requests_minimal_projection_and_honest_local_filter() {
    let (core, context, _) = setup(ReadOperation::GmailListMetadata);
    let transport = FixtureTransport::new(vec![
        json!({"messages":[{"id":"abc1"},{"id":"abc2"}]}),
        message("abc1", -60),
        message("abc2", -90000),
    ]);
    let result = dispatch(&core, context.clone(), &transport, "fixture-client")
        .await
        .unwrap();
    let value = safe_result(&result);
    assert_eq!(value["kind"], "gmailMetadata");
    assert_eq!(value["items"].as_array().unwrap().len(), 1);
    assert_eq!(value["items"][0]["subject"], "Synthetic café");
    assert_eq!(
        value["items"][0]["sourceUrl"],
        "https://mail.google.com/mail/u/0/#inbox/abc1"
    );
    assert_eq!(value["truncated"], false);
    assert_eq!(value["partial"], false);
    assert_eq!(value["expiresAt"], context.expires_at);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[0].url.as_str().split('?').next().unwrap(),
        GMAIL_LIST
    );
    assert_eq!(
        pairs(&requests[0].url),
        vec![
            ("labelIds".into(), "INBOX".into()),
            ("includeSpamTrash".into(), "false".into()),
            ("maxResults".into(), "100".into()),
            ("fields".into(), GMAIL_LIST_FIELDS.into())
        ]
    );
    assert_eq!(
        pairs(&requests[1].url),
        vec![
            ("format".into(), "metadata".into()),
            ("metadataHeaders".into(), "From".into()),
            ("metadataHeaders".into(), "Subject".into()),
            ("fields".into(), GMAIL_FIELDS.into())
        ]
    );
    for request in requests.iter() {
        assert!(request.refresh_form.is_none());
        assert_eq!(
            request.access.as_ref().unwrap().as_str(),
            "fixture-access-secret"
        );
        assert!(!pairs(&request.url).iter().any(|(k, _)| k == "q"));
        assert!(!request.url.as_str().contains("secret"));
    }
}
#[tokio::test]
async fn gmail_result_cap_stops_metadata_calls_and_reports_incomplete_scan() {
    let (core, mut context, _) = setup(ReadOperation::GmailListMetadata);
    context.args["maxItems"] = json!(1);
    let transport = FixtureTransport::new(vec![
        json!({"messages":[{"id":"a1"},{"id":"a2"}]}),
        message("a1", -60),
    ]);
    let result = dispatch(&core, context, &transport, "fixture-client")
        .await
        .unwrap();
    let value = safe_result(&result);
    assert_eq!(value["truncated"], true);
    assert_eq!(value["partial"], true);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn gmail_page_bound_is_not_an_empty_complete_inbox_claim() {
    let (core, context, _) = setup(ReadOperation::GmailListMetadata);
    let transport = FixtureTransport::new(vec![
        json!({"nextPageToken":"one"}),
        json!({"nextPageToken":"two"}),
        json!({"nextPageToken":"three"}),
    ]);
    let result = dispatch(&core, context, &transport, "fixture-client")
        .await
        .unwrap();
    assert!(result.truncated && result.partial);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 3);
}
#[tokio::test]
async fn gmail_scan_count_is_bounded_even_when_date_filter_excludes_everything() {
    let (core, context, _) = setup(ReadOperation::GmailListMetadata);
    let ids: Vec<_> = (0..100).map(|i| format!("a{i:x}")).collect();
    let mut responses = vec![
        json!({"messages":ids.iter().map(|id| json!({"id":id})).collect::<Vec<_>>(),"nextPageToken":"more"}),
    ];
    responses.extend(ids.iter().map(|id| message(id, -90000)));
    let transport = FixtureTransport::new(responses);
    let result = dispatch(&core, context, &transport, "fixture-client")
        .await
        .unwrap();
    assert!(result.truncated && result.partial);
    assert_eq!(safe_result(&result)["items"].as_array().unwrap().len(), 0);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 101);
}
#[tokio::test]
async fn calendar_fixed_primary_fields_window_pagination_and_all_day_semantics() {
    let (core, mut context, _) = setup(ReadOperation::CalendarListEvents);
    context.args =
        json!({"startAt":"2026-09-01T00:00:00Z","endAt":"2026-09-08T00:00:00Z","maxItems":2});
    let transport = FixtureTransport::new(vec![
        json!({"items":[{"id":"event_1","summary":"Synthetic day","start":{"date":"2026-09-02"},"end":{"date":"2026-09-03"},"status":"confirmed","description":"BODY_MUST_NOT_LEAVE","attendees":[{"email":"SECRET_ATTACHMENT"}]}],"nextPageToken":"opaque+/page"}),
        json!({"items":[{"id":"event2","summary":"Meeting","start":{"dateTime":"2026-09-04T12:00:00Z"},"end":{"dateTime":"2026-09-04T13:00:00Z"},"status":"tentative"}],"nextPageToken":"more"}),
    ]);
    let result = dispatch(&core, context, &transport, "fixture-client")
        .await
        .unwrap();
    let value = safe_result(&result);
    assert_eq!(value["kind"], "calendarEvents");
    assert_eq!(value["items"][0]["startAt"], "2026-09-02");
    assert_eq!(value["items"][0]["endAt"], "2026-09-03");
    assert_eq!(value["items"][0]["allDay"], true);
    assert_eq!(value["items"][1]["allDay"], false);
    assert!(result.truncated);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url.as_str().split('?').next().unwrap(),
        CALENDAR_LIST
    );
    assert_eq!(
        pairs(&requests[0].url),
        vec![
            ("timeMin".into(), "2026-09-01T00:00:00Z".into()),
            ("timeMax".into(), "2026-09-08T00:00:00Z".into()),
            ("maxResults".into(), "2".into()),
            ("singleEvents".into(), "true".into()),
            ("orderBy".into(), "startTime".into()),
            ("showDeleted".into(), "false".into()),
            ("fields".into(), CALENDAR_FIELDS.into())
        ]
    );
    assert!(pairs(&requests[1].url).contains(&("maxResults".into(), "1".into())));
    assert!(pairs(&requests[1].url).contains(&("pageToken".into(), "opaque+/page".into())));
}
fn calendar_contract_fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../runtime/tests/fixtures/calendar-native-results.json"
    )))
    .unwrap()
}

#[tokio::test]
async fn read_request_shared_utc_microsecond_and_exact_window_bounds() {
    let fixture = calendar_contract_fixture();
    for case in fixture["requestCases"].as_array().unwrap() {
        let (core, mut context, _) = setup(ReadOperation::CalendarListEvents);
        context.args = case["args"].clone();
        let transport = FixtureTransport::new(vec![json!({"items": []})]);
        let result = dispatch(&core, context, &transport, "fixture-client").await;
        assert_eq!(
            result.is_ok(),
            case["accepted"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            usize::from(case["accepted"] == true)
        );
    }
}

#[tokio::test]
async fn gmail_shared_microsecond_window_edges_do_not_truncate_to_milliseconds() {
    let fixture = calendar_contract_fixture();
    for case in fixture["gmailMicrosecondCases"].as_array().unwrap() {
        let (core, mut context, _) = setup(ReadOperation::GmailListMetadata);
        let base =
            chrono::DateTime::from_timestamp_millis(chrono::Utc::now().timestamp_millis() - 60_000)
                .unwrap();
        let start =
            base + chrono::Duration::microseconds(case["startOffsetMicros"].as_i64().unwrap());
        let end = base + chrono::Duration::microseconds(case["endOffsetMicros"].as_i64().unwrap());
        context.args = json!({"startAt": start.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            "endAt": end.to_rfc3339_opts(chrono::SecondsFormat::Micros, true), "maxItems": 100});
        let records = case["records"].as_array().unwrap();
        let mut responses = vec![
            json!({"messages": records.iter().map(|r| json!({"id": r["id"]})).collect::<Vec<_>>()}),
        ];
        let mut expected = Vec::new();
        for record in records {
            let id = record["id"].as_str().unwrap();
            let mut metadata = message(id, -60);
            metadata["internalDate"] = json!((base.timestamp_millis()
                + record["offsetMillis"].as_i64().unwrap())
            .to_string());
            responses.push(metadata);
            if record["included"] == true {
                expected.push(record["id"].clone());
            }
        }
        let transport = FixtureTransport::new(responses);
        let result = dispatch(&core, context, &transport, "fixture-client")
            .await
            .unwrap();
        let value = safe_result(&result);
        let actual: Vec<_> = value["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].clone())
            .collect();
        assert_eq!(actual, expected, "{}", case["name"]);
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1 + records.len());
    }
}

#[tokio::test]
async fn calendar_native_projection_matches_shared_python_validator_fixtures() {
    let fixture = calendar_contract_fixture();
    for case in fixture["cases"].as_array().unwrap() {
        let (core, mut context, _) = setup(ReadOperation::CalendarListEvents);
        context.args = case.get("args").unwrap_or(&fixture["args"]).clone();
        let transport = FixtureTransport::new(vec![json!({"items": [case["providerEvent"]]})]);
        let result = dispatch(&core, context.clone(), &transport, "fixture-client").await;
        if case["accepted"] == true {
            let result = result.unwrap_or_else(|e| panic!("{}: {}", case["name"], e.code));
            let expected = json!({
                "kind": "calendarEvents",
                "retrievedAt": result.retrieved_at,
                "expiresAt": context.expires_at,
                "startAt": context.args["startAt"],
                "endAt": context.args["endAt"],
                "truncated": false,
                "partial": false,
                "items": [case["nativeItem"]]
            });
            assert_eq!(safe_result(&result), expected, "{}", case["name"]);
        } else if case["filtered"] == true {
            assert_eq!(
                safe_result(&result.unwrap())["items"],
                json!([]),
                "{}",
                case["name"]
            );
        } else {
            assert!(result.is_err(), "{}", case["name"]);
        }
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn calendar_shared_fixture_count_and_byte_limits() {
    let fixture = calendar_contract_fixture();
    for case in fixture["sizeCases"].as_array().unwrap() {
        let (core, mut context, _) = setup(ReadOperation::CalendarListEvents);
        context.args = fixture["args"].clone();
        let mut provider_items = Vec::new();
        let mut native_items = Vec::new();
        for index in 0..case["count"].as_u64().unwrap() {
            let mut provider = fixture["cases"][0]["providerEvent"].clone();
            let mut native = fixture["cases"][0]["nativeItem"].clone();
            let id = json!(match case["idLength"].as_u64() {
                Some(length) => "a".repeat(length as usize),
                None => format!("size_{index}"),
            });
            provider["id"] = id.clone();
            native["id"] = id;
            if let Some(repeat) = case["titleRepeat"].as_u64() {
                let title = json!(case["titleCharacter"]
                    .as_str()
                    .unwrap()
                    .repeat(repeat as usize));
                provider["summary"] = title.clone();
                native["title"] = title;
            }
            provider_items.push(provider);
            native_items.push(native);
        }
        let transport = FixtureTransport::new(vec![json!({"items": provider_items})]);
        let result = dispatch(&core, context, &transport, "fixture-client").await;
        if case["accepted"] == true {
            assert_eq!(
                safe_result(&result.unwrap())["items"],
                json!(native_items),
                "{}",
                case["name"]
            );
        } else {
            assert!(result.is_err(), "{}", case["name"]);
        }
    }
}

#[tokio::test]
async fn no_network_for_wrong_scope_identity_generation_egress_expiry_or_unknown_args() {
    let (core, context, _) = setup(ReadOperation::GmailListMetadata);
    let mut variants = Vec::new();
    macro_rules! changed {
        ($field:ident, $value:expr) => {{
            let mut c = context.clone();
            c.$field = $value;
            variants.push(c);
        }};
    }
    changed!(workspace_id, "another".into());
    changed!(device_id, "another".into());
    changed!(run_id, "../escape".into());
    changed!(connection_id, uuid::Uuid::new_v4().to_string());
    changed!(grant_id, uuid::Uuid::new_v4().to_string());
    changed!(generation, 2);
    changed!(runtime_origin, "https://evil.test".into());
    changed!(model_origin, "https://evil.test".into());
    changed!(expires_at, at(-1));
    changed!(expires_at, at(300));
    changed!(operation, ReadOperation::CalendarListEvents);
    for key in ["q", "body", "attachments", "url", "calendarId"] {
        let mut c = context.clone();
        c.args[key] = json!("forbidden");
        variants.push(c);
    }
    for count in [0, 101] {
        let mut c = context.clone();
        c.args["maxItems"] = json!(count);
        variants.push(c);
    }
    let mut c = context.clone();
    c.args["startAt"] = json!(at(-8 * 86400));
    variants.push(c);
    let transport = FixtureTransport::default();
    for context in variants {
        assert!(dispatch(&core, context, &transport, "fixture-client")
            .await
            .is_err());
    }
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    let mut status = store_guard(&core)
        .unwrap()
        .get(&context.connection_id)
        .unwrap()
        .unwrap();
    status.scopes = vec![ALLOWED_SCOPES[2].into()];
    store_guard(&core).unwrap().upsert(&status).unwrap();
    assert!(dispatch(&core, context, &transport, "fixture-client")
        .await
        .is_err());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn grants_require_explicit_exact_categories_origins_operations_finite_retention_consent() {
    let (core, context, _) = setup(ReadOperation::GmailListMetadata);
    let status = store_guard(&core)
        .unwrap()
        .get(&context.connection_id)
        .unwrap()
        .unwrap();
    let template = lifecycle_guard(&core)
        .unwrap()
        .grants
        .list(chrono::Utc::now().timestamp_millis())[0]
        .scope
        .clone();
    let mut requests = Vec::new();
    let mut r = template.clone();
    r.account_read_consented = false;
    requests.push(r);
    let mut r = template.clone();
    r.egress.consented = false;
    requests.push(r);
    let mut r = template.clone();
    r.egress.retention = "forever".into();
    requests.push(r);
    let mut r = template.clone();
    r.egress.data_categories = vec!["gmail.bodies".into()];
    requests.push(r);
    let mut r = template.clone();
    r.operations = vec![ReadOperation::GmailListMetadata; 2];
    requests.push(r);
    let mut r = template.clone();
    r.expires_at = at(3601);
    requests.push(r);
    let mut r = template.clone();
    r.expires_at = at(-1);
    requests.push(r);
    for origin in [
        "http://public.example.test",
        "https://user:secret@model.test",
        "https://model.test/v1",
        "https://model.test/",
        "https://model.test?secret=x",
    ] {
        let mut r = template.clone();
        r.egress.model_origin = origin.into();
        requests.push(r);
    }
    for request in requests {
        assert!(lifecycle_guard(&core)
            .unwrap()
            .grants
            .register(request, &status, chrono::Utc::now().timestamp_millis())
            .is_err());
    }
    let mut wire = serde_json::to_value(template).unwrap();
    wire["grantId"] = json!(uuid::Uuid::new_v4().to_string());
    assert!(serde_json::from_value::<GrantRequest>(wire).is_err());
    assert!(serde_json::from_value::<ReadOperation>(json!("forma_gmail_send")).is_err());
}
#[tokio::test]
async fn revoke_disconnect_cancel_and_scope_change_reject_late_success() {
    for action in 0..4 {
        let (core, context, secrets) = setup(ReadOperation::GmailListMetadata);
        let transport = FixtureTransport::new(vec![json!({"messages":[]})]);
        let c = core.clone();
        let x = context.clone();
        *transport.hook.lock().unwrap() = Some(Box::new(move || match action {
            0 => lifecycle_guard(&c).unwrap().grants.revoke(&x.grant_id),
            1 => disconnect_impl(&c, &x.connection_id).unwrap(),
            2 => lifecycle_guard(&c)
                .unwrap()
                .grants
                .cancel_run(
                    &x.workspace_id,
                    &x.run_id,
                    chrono::Utc::now().timestamp_millis(),
                )
                .unwrap(),
            _ => {
                let mut status = store_guard(&c)
                    .unwrap()
                    .get(&x.connection_id)
                    .unwrap()
                    .unwrap();
                status.scopes = vec![ALLOWED_SCOPES[2].into()];
                store_guard(&c).unwrap().upsert(&status).unwrap();
            }
        }));
        assert!(
            dispatch(&core, context.clone(), &transport, "fixture-client")
                .await
                .is_err()
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        if action == 1 {
            assert!(secrets.0.lock().unwrap().is_empty());
        }
        assert!(dispatch(&core, context, &transport, "fixture-client")
            .await
            .is_err());
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn single_flight_drop_releases_read_lease_and_cancel_fences_future_requests() {
    let (core, context, _) = setup(ReadOperation::GmailListMetadata);
    let first = ReadLease::begin(&core, context.clone()).unwrap();
    assert!(ReadLease::begin(&core, context.clone()).is_err());
    drop(first);
    let second = ReadLease::begin(&core, context.clone()).unwrap();
    lifecycle_guard(&core)
        .unwrap()
        .grants
        .cancel_run(
            &context.workspace_id,
            &context.run_id,
            chrono::Utc::now().timestamp_millis(),
        )
        .unwrap();
    assert!(second.cancel.is_cancelled());
    assert!(second.check().is_err());
    drop(second);
    assert!(ReadLease::begin(&core, context).is_err());
}
fn expire_credential(core: &ConnectorsCore, id: &str) {
    let mut status = store_guard(core).unwrap().get(id).unwrap().unwrap();
    status.expires_at = at(-1);
    store_guard(core).unwrap().upsert(&status).unwrap();
}
fn refreshed(scope: Option<&str>) -> serde_json::Value {
    let mut response = json!({"access_token":"fixture-rotated-access","refresh_token":"fixture-rotated-refresh","expires_in":3600,"token_type":"Bearer"});
    if let Some(scope) = scope {
        response["scope"] = json!(scope);
    }
    response
}
#[tokio::test]
async fn refresh_if_needed_uses_fixed_native_post_and_new_access_never_in_result() {
    let (core, context, secrets) = setup(ReadOperation::GmailListMetadata);
    expire_credential(&core, &context.connection_id);
    let transport = FixtureTransport::new(vec![refreshed(None), json!({"messages":[]})]);
    let result = dispatch(&core, context.clone(), &transport, "fixture-client")
        .await
        .unwrap();
    let serialized = safe_result(&result).to_string();
    assert!(!serialized.contains("fixture-rotated"));
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests[0].url.as_str(), oauth::TOKEN_ENDPOINT);
    assert!(requests[0].access.is_none());
    let form = requests[0].refresh_form.as_ref().unwrap();
    assert_eq!(
        form.iter()
            .map(|(k, v)| (*k, v.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("grant_type", "refresh_token"),
            ("client_id", "fixture-client"),
            ("refresh_token", "fixture-refresh-secret")
        ]
    );
    assert_eq!(
        requests[1].access.as_ref().unwrap().as_str(),
        "fixture-rotated-access"
    );
    assert!(secrets
        .read(&context.connection_id)
        .unwrap()
        .contains("fixture-rotated-refresh"));
}
#[tokio::test]
async fn refresh_disconnect_race_never_recreates_credentials_and_narrowing_revokes_reads() {
    for disconnect in [true, false] {
        let (core, context, secrets) = setup(ReadOperation::GmailListMetadata);
        expire_credential(&core, &context.connection_id);
        let transport = FixtureTransport::new(vec![refreshed(if disconnect {
            None
        } else {
            Some(ALLOWED_SCOPES[2])
        })]);
        if disconnect {
            let c = core.clone();
            let id = context.connection_id.clone();
            *transport.hook.lock().unwrap() =
                Some(Box::new(move || disconnect_impl(&c, &id).unwrap()));
        }
        assert!(dispatch(&core, context, &transport, "fixture-client")
            .await
            .is_err());
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        if disconnect {
            assert!(secrets.0.lock().unwrap().is_empty());
        }
        assert!(lifecycle_guard(&core)
            .unwrap()
            .grants
            .list(chrono::Utc::now().timestamp_millis())
            .is_empty());
    }
}
#[tokio::test]
async fn malformed_oversized_replayed_and_unauthorized_responses_never_deliver_data() {
    let (core, context, _) = setup(ReadOperation::GmailListMetadata);
    for responses in [
        vec![json!({"messages":[{"id":"../../token"}]})],
        vec![json!({"messages":[{"id":"abc"}]}), message("def", -60)],
        vec![
            json!({"nextPageToken":"same"}),
            json!({"nextPageToken":"same"}),
        ],
        vec![
            json!({"messages":[{"id":"abc"}]}),
            json!({"id":"abc","internalDate":"bad","labelIds":[],"payload":{}}),
        ],
        vec![json!({"huge":"x".repeat(MAX_BODY_BYTES+1)})],
    ] {
        assert!(dispatch(
            &core,
            context.clone(),
            &FixtureTransport::new(responses),
            "fixture-client"
        )
        .await
        .is_err());
    }
    let transport = FixtureTransport::default();
    transport
        .responses
        .lock()
        .unwrap()
        .push_back(Err(AppError::new(
            "connector_auth",
            "Account access was refused. Reconnect the account.",
        )));
    let error = dispatch(&core, context, &transport, "fixture-client")
        .await
        .unwrap_err();
    assert_eq!(error.code, "connector_auth");
    assert!(!serde_json::to_string(&error).unwrap().contains("secret"));
}
struct PendingTransport {
    cancel: CancellationToken,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}
struct PendingGuard(Arc<std::sync::atomic::AtomicBool>);
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl Transport for PendingTransport {
    fn send(
        &self,
        request: Request,
    ) -> Pin<Box<dyn Future<Output = AppResult<Zeroizing<Vec<u8>>>> + Send + '_>> {
        Box::pin(async move {
            let _guard = PendingGuard(self.dropped.clone());
            let _credential_lease = request;
            self.cancel.cancel();
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn cancellation_drops_pending_transport_and_native_credential_lease() {
    let (core, context, _) = setup(ReadOperation::GmailListMetadata);
    let lease = ReadLease::begin(&core, context).unwrap();
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let transport = PendingTransport {
        cancel: lease.cancel.clone(),
        dropped: dropped.clone(),
    };
    let result = lease
        .request(
            &transport,
            Request {
                url: reqwest::Url::parse(GMAIL_LIST).unwrap(),
                access: Some(Zeroizing::new("fixture-access-secret".into())),
                refresh_form: None,
            },
        )
        .await;
    assert_eq!(result.unwrap_err().code, "connector_stale");
    assert!(dropped.load(Ordering::SeqCst));
}
#[test]
fn expiry_revocation_restart_and_refresh_authorization_are_fenced() {
    let (core, context, _) = setup(ReadOperation::GmailListMetadata);
    let lease = ReadLease::begin(&core, context.clone()).unwrap();
    // Expire through the same path used by command snapshots; no wall-clock sleep.
    assert!(lifecycle_guard(&core)
        .unwrap()
        .grants
        .list(chrono::Utc::now().timestamp_millis() + 601_000)
        .is_empty());
    assert!(lease.cancel.is_cancelled());
    assert!(
        begin_refresh_checked(&core, &context.connection_id, Some((&context, &lease.id))).is_err()
    );
    assert!(lease.check().is_err());
    let mut restarted = grants::Grants::default();
    assert!(restarted
        .check(&context, chrono::Utc::now().timestamp_millis())
        .is_err());
}
#[test]
fn revoked_read_cannot_commit_a_previously_started_refresh() {
    let (core, context, _) = setup(ReadOperation::GmailListMetadata);
    let lease = ReadLease::begin(&core, context.clone()).unwrap();
    let (refresh_lease, refresh) =
        begin_refresh_checked(&core, &context.connection_id, Some((&context, &lease.id))).unwrap();
    lifecycle_guard(&core)
        .unwrap()
        .grants
        .revoke(&context.grant_id);
    let tokens =
        oauth::parse_token_response(&serde_json::to_vec(&refreshed(None)).unwrap()).unwrap();
    assert!(commit_refresh_checked(
        &refresh_lease,
        &refresh,
        tokens,
        Some((&context, &lease.id))
    )
    .is_err());
    assert_eq!(
        read_stored(&core, &context.connection_id)
            .unwrap()
            .access_token,
        "fixture-access-secret"
    );
}
#[test]
fn live_gate_cannot_be_lifted_by_a_configured_client_or_grant() {
    let error = live_gate().unwrap_err();
    assert!(matches!(
        error.code,
        "connector_disabled" | "connector_live_gate"
    ));
}
