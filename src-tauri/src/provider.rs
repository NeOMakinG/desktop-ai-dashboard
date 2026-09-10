use crate::{types::*, validation};
use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Client, RequestBuilder,
};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use zeroize::Zeroizing;

pub trait Credentials: Send {
    fn read(&self, reference: &str) -> AppResult<Zeroizing<String>>;
    fn write(&self, reference: &str, secret: &str) -> AppResult<()>;
    fn remove(&self, reference: &str) -> AppResult<()>;
}
pub struct OsCredentials {
    service: String,
}
impl OsCredentials {
    pub fn new(test_directory: Option<&Path>) -> Self {
        let service = match test_directory {
            Some(path) => format!(
                "dev.forma.workspace.qa.{:x}",
                Sha256::digest(path.as_os_str().to_string_lossy().as_bytes())
            ),
            None => "dev.forma.workspace.credentials.v1".to_owned(),
        };
        Self { service }
    }
    fn entry(&self, reference: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(&self.service, reference).map_err(|_| credential_error())
    }
}
fn credential_error() -> AppError {
    AppError::new("credential_store", "The OS credential store is unavailable or access was denied. Unlock it and reconnect your AI provider. No key is saved in plaintext.")
}
impl Credentials for OsCredentials {
    fn read(&self, reference: &str) -> AppResult<Zeroizing<String>> {
        self.entry(reference)?
            .get_password()
            .map(Zeroizing::new)
            .map_err(|_| credential_error())
    }
    fn write(&self, reference: &str, secret: &str) -> AppResult<()> {
        self.entry(reference)?
            .set_password(secret)
            .map_err(|_| credential_error())
    }
    fn remove(&self, reference: &str) -> AppResult<()> {
        match self.entry(reference)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(credential_error()),
        }
    }
}
pub fn credential_reference(endpoint: &str) -> String {
    format!(
        "{:x}:{}",
        Sha256::digest(endpoint.as_bytes()),
        uuid::Uuid::new_v4()
    )
}
pub fn credential_matches(endpoint: &str, reference: &str) -> bool {
    reference.split_once(':').is_some_and(|(digest, id)| {
        digest == format!("{:x}", Sha256::digest(endpoint.as_bytes())) && validation::id(id).is_ok()
    })
}

struct SafeResolver;
impl reqwest::dns::Resolve for SafeResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .take(65)
                .collect();
            if addresses.is_empty()
                || addresses.len() > 64
                || addresses
                    .iter()
                    .any(|address| !validation::allowed_address(address.ip()))
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "Unsupported provider address",
                )
                .into());
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

pub fn client() -> AppResult<Client> {
    Client::builder()
        .dns_resolver(std::sync::Arc::new(SafeResolver))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(45))
        .pool_max_idle_per_host(2)
        .build()
        .map_err(|_| AppError::new("network", "The native network client could not start."))
}
fn authorize(request: RequestBuilder, secret: Option<&str>) -> AppResult<RequestBuilder> {
    match secret {
        None => Ok(request),
        Some(secret) => {
            validation::api_key(secret)?;
            let mut header = HeaderValue::from_str(&format!("Bearer {secret}"))
                .map_err(|_| AppError::invalid())?;
            header.set_sensitive(true);
            Ok(request.header(AUTHORIZATION, header))
        }
    }
}
fn network_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        AppError::new(
            "timeout",
            "The provider timed out. Your history is preserved; try again when ready.",
        )
    } else {
        AppError::new(
            "network",
            "Could not reach the provider. Check the connection and endpoint.",
        )
    }
}
async fn response(request: RequestBuilder) -> AppResult<Value> {
    let mut response = request.send().await.map_err(network_error)?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => AppError::new("provider_auth", "The provider rejected access. Check the API key and permissions."),
            429 => AppError::new("provider_rate_limit", "The provider is rate-limiting requests. Try again later."),
            300..=399 => AppError::new("provider_redirect", "The provider redirected this request. Configure its final API endpoint; redirects are not followed."),
            _ => AppError::new("provider_http", "The provider rejected the request. Check the endpoint, model, and provider availability."),
        });
    }
    if response
        .content_length()
        .is_some_and(|n| n > validation::MAX_RESPONSE as u64)
    {
        return Err(malformed());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len().saturating_add(chunk.len()) > validation::MAX_RESPONSE {
            return Err(malformed());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| malformed())
}
fn malformed() -> AppError {
    AppError::new(
        "provider_response",
        "The provider returned an unsupported, empty, or oversized response.",
    )
}
fn contains_secret(value: &str, key: Option<&str>) -> bool {
    key.is_some_and(|key| !key.is_empty() && value.contains(key))
}
pub fn parse_models(body: &Value, key: Option<&str>) -> AppResult<Vec<String>> {
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(malformed)?;
    if data.len() > 200 {
        return Err(malformed());
    }
    let mut models = Vec::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(malformed)?;
        validation::model(id, false).map_err(|_| malformed())?;
        if contains_secret(id, key) {
            return Err(malformed());
        }
        if !models.iter().any(|model| model == id) {
            models.push(id.to_owned());
        }
    }
    Ok(models)
}
// Rank version components, never date suffixes. Unknown IDs stay in provider order.
fn opus_version(id: &str) -> Option<(u32, u32)> {
    let version = id.strip_prefix("claude-opus-")?;
    let mut parts = version.split('-');
    let component = |part: &str| {
        if part.len() <= 3 && part.bytes().all(|b| b.is_ascii_digit()) {
            part.parse::<u32>().ok()
        } else {
            None
        }
    };
    let major = component(parts.next()?)?;
    let minor = parts.next().and_then(component).unwrap_or(0);
    Some((major, minor))
}

pub fn selected_model<'a>(configured: &'a str, models: &'a [String]) -> AppResult<&'a str> {
    if !configured.is_empty() {
        return models.iter().find(|id| id.as_str() == configured)
            .map(String::as_str)
            .ok_or_else(|| AppError::new("model_unavailable", "The selected model was not returned by this provider. Choose an available model and check again."));
    }
    let mut preferred: Option<(&str, (u32, u32))> = None;
    for id in models {
        if let Some(version) = opus_version(id) {
            // Equal versions retain discovery order, rather than guessing alias semantics.
            if preferred.is_none_or(|(_, current)| version > current) {
                preferred = Some((id, version));
            }
        }
    }
    preferred.map(|(id, _)| id).or_else(|| models.first().map(String::as_str))
        .ok_or_else(|| AppError::new("models_empty", "This provider returned no models. Refresh the list or check your connection settings."))
}

pub async fn check(client: &Client, base: &str, key: Option<&str>) -> AppResult<Vec<String>> {
    let request = authorize(client.get(format!("{base}/models")), key)?;
    parse_models(&response(request).await?, key)
}
#[cfg(test)]
pub fn parse_reply(body: &Value, key: Option<&str>) -> AppResult<String> {
    let choices = body
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(malformed)?;
    if choices.len() != 1 {
        return Err(malformed());
    }
    let choice = &choices[0];
    if choice.get("finish_reason").and_then(Value::as_str) != Some("stop") {
        return Err(AppError::new("provider_incomplete", "The provider did not return a complete text reply. Your message is preserved; try a shorter request."));
    }
    let message = choice.get("message").ok_or_else(malformed)?;
    if message.get("role").and_then(Value::as_str) != Some("assistant")
        || message.get("tool_calls").is_some_and(|v| !v.is_null())
        || message.get("function_call").is_some_and(|v| !v.is_null())
    {
        return Err(malformed());
    }
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(malformed)?;
    validation::text(content, validation::MAX_TEXT, false).map_err(|_| malformed())?;
    if contains_secret(content, key) {
        return Err(malformed());
    }
    let canonical = crate::hermes::canonical_reply(content, key).ok_or_else(malformed)?;
    if contains_secret(&canonical, key) {
        return Err(malformed());
    }
    // Adding omitted actions must not grow a valid reply past the native limit.
    Ok(if canonical.len() > validation::MAX_TEXT {
        content.to_owned()
    } else {
        canonical
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reply_boundary_canonicalizes_only_host_validated_components() {
        let reply = |content: &str| json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":content}}]});
        let raw = " {\"blocks\":[{\"body\":\"<b>plain</b>\",\"title\":\"t\",\"type\":\"card\"}],\"kind\":\"components\"} ";
        let result = parse_reply(&reply(raw), None).unwrap();
        assert_eq!(
            crate::hermes::validate(&result),
            crate::hermes::validate(raw)
        );
        assert!(serde_json::from_str::<Value>(&result).unwrap()["blocks"][0]["actions"].is_array());
        for raw in [
            "  ordinary <b>text</b>\n",
            "```json\n{}\n```",
            "{\"kind\":\"components\",\"blocks\":[{\"type\":\"script\"}]}",
        ] {
            assert_eq!(parse_reply(&reply(raw), None).unwrap(), raw);
        }
    }

    #[test]
    fn canonicalization_does_not_expand_native_reply_limit() {
        let mut blocks = vec![json!({"type":"card","title":"t","body":"b".repeat(700)}); 40];
        let base_size = json!({"kind":"components","blocks":blocks})
            .to_string()
            .len();
        let extra = validation::MAX_TEXT - base_size;
        blocks[0]["body"] = json!("b".repeat(700 + extra));
        let raw = json!({"kind":"components","blocks":blocks}).to_string();
        assert_eq!(raw.len(), validation::MAX_TEXT);
        assert!(matches!(
            crate::hermes::validate(&raw),
            crate::hermes::Validated::Components(_)
        ));
        assert!(crate::hermes::canonical_reply(&raw, None).unwrap().len() > validation::MAX_TEXT);
        let reply = json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":raw}}]});
        assert_eq!(parse_reply(&reply, None).unwrap(), raw);
    }

    #[test]
    fn numeric_overflow_duplicate_with_escaped_key_is_rejected_before_storage() {
        let fixtures: Vec<Value> =
            serde_json::from_str(include_str!("../../src/app/message-conformance.json")).unwrap();
        let raw = fixtures
            .iter()
            .find(|fixture| {
                fixture["name"] == "overwritten numeric overflow with escaped synthetic key"
            })
            .unwrap()["input"]
            .as_str()
            .unwrap();
        assert!(!raw.contains("secret"), "synthetic key is JSON-escaped");
        assert_eq!(
            crate::hermes::validate(raw),
            crate::hermes::Validated::Fallback
        );
        let reply = json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":raw}}]});
        assert_eq!(
            parse_reply(&reply, Some("secret")).unwrap_err().code,
            "provider_response"
        );
        assert_eq!(parse_reply(&reply, None).unwrap(), raw);
        assert_eq!(
            parse_reply(&reply, Some("unrelated-credential")).unwrap(),
            raw
        );
        // TS still rejects the shape and displays escaped raw text when given
        // this fixture directly; credential authority belongs to the native host.
    }

    #[test]
    fn escaped_fallback_credentials_are_rejected_without_changing_benign_raw_text() {
        let reply = |content: &str| json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":content}}]});
        let templates = [
            r#"{"text":"SECRET_SLOT"#,
            r#"{"text":"SECRET_SLOT",}"#,
            r#"{"kind":"other","blocks":[{"type":"markdown","text":"SECRET_SLOT"}]}"#,
            r#"{"kind":"components","blocks":[{"type":"unknown","text":"SECRET_SLOT"}]}"#,
            r#"{"kind":"components","blocks":[{"type":"markdown","text":{"extra":"SECRET_SLOT"}}]}"#,
            r#"{"kind":"components","blocks":[],"SECRET_SLOT":"extra key"}"#,
            r#"{"kind":"components","blocks":[]} trailing "SECRET_SLOT""#,
            r#"{"text":"\q \uD800 \uNOPE SECRET_SLOT"}"#,
            r#"Ordinary escaped prose: SECRET_SLOT."#,
        ];
        for (key, escaped) in [
            ("secret", r#"\u0073ecret"#),
            ("sëcret🧡", r#"\u0073ëcr\u0065t\uD83E\uDDE1"#),
            ("sëcret🧡", r#"\u0073ëcret🧡"#),
            ("se\"cr\\et", r#"se\"cr\\et"#),
            ("🧡\"\\", r#"\uD83E\uDDE1\"\\"#),
        ] {
            for template in templates {
                let raw = template.replace("SECRET_SLOT", escaped);
                assert!(!raw.contains(key), "fixture must bypass the literal guard");
                assert_eq!(
                    crate::hermes::validate(&raw),
                    crate::hermes::Validated::Fallback
                );
                let body = reply(&raw);
                let error = parse_reply(&body, Some(key)).unwrap_err();
                assert_eq!(error.code, "provider_response");
                assert_eq!(error.message, malformed().message);
                for absent in [None, Some(""), Some("unrelated-credential")] {
                    assert_eq!(parse_reply(&body, absent).unwrap(), raw);
                }
            }
        }
        for raw in [
            "  Ordinary text.\n",
            r#"  Prose: \u0068ello \"quoted\" C:\\temp\\file \uD83E\uDDE1  "#,
            r#"{"broken":"\q \uNOPE \uD800 \uDDE1 \u123"#,
            r#"{"kind":"components","blocks":[{"type":"markdown","text":"sec","other":"ret"}]}"#,
        ] {
            assert_eq!(parse_reply(&reply(raw), Some("secret")).unwrap(), raw);
        }
    }

    #[test]
    fn fallback_credential_scan_reaches_native_text_boundary_without_expanding_it() {
        let suffix = "\\u0073ecret";
        let raw = format!(
            "{}{suffix}",
            "x".repeat(validation::MAX_TEXT - suffix.len())
        );
        assert_eq!(raw.len(), validation::MAX_TEXT);
        let reply = |content: &str| json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":content}}]});
        assert_eq!(
            parse_reply(&reply(&raw), Some("secret")).unwrap_err().code,
            "provider_response"
        );
        assert_eq!(
            parse_reply(&reply(&raw), Some("unrelated-credential")).unwrap(),
            raw
        );
        assert!(parse_reply(&reply(&(raw + "x")), None).is_err());
    }

    #[test]
    fn escaped_credentials_in_overwritten_fields_are_rejected_before_canonicalization() {
        for raw in [
            r#"{"kind":"components","blocks":[{"type":"markdown","text":"\u0073ecret","text":"safe"}]}"#,
            r#"{"kind":"\u0073ecret","kind":"components","blocks":[]}"#,
            r#"{"kind":"components","blocks":[{"type":"unknown","text":"\u0073ecret"}],"blocks":[]}"#,
        ] {
            assert!(matches!(
                crate::hermes::validate(raw),
                crate::hermes::Validated::Components(_)
            ));
            let body = json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":raw}}]});
            assert_eq!(
                parse_reply(&body, Some("secret")).unwrap_err().code,
                "provider_response"
            );
            assert!(parse_reply(&body, Some("unrelated-credential")).is_ok());
        }
    }

    #[test]
    fn decoded_component_credentials_never_reach_persistence_or_renderer() {
        let reply = |content: &str| json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":content}}]});
        let templates = [
            json!({"type":"markdown","text":"SECRET_SLOT"}),
            json!({"type":"card","title":"SECRET_SLOT","body":"body"}),
            json!({"type":"card","title":"title","body":"SECRET_SLOT"}),
            json!({"type":"card","title":"title","body":"body","actions":[{"label":"SECRET_SLOT","href":"https://example.com"}]}),
            json!({"type":"list","items":[{"title":"SECRET_SLOT","detail":"detail","icon":"globe"}]}),
            json!({"type":"list","items":[{"title":"title","detail":"SECRET_SLOT","icon":"globe"}]}),
            json!({"type":"kv","rows":[{"label":"SECRET_SLOT","value":"value"}]}),
            json!({"type":"kv","rows":[{"label":"label","value":"SECRET_SLOT"}]}),
            json!({"type":"callout","tone":"info","text":"SECRET_SLOT"}),
        ];
        for key in ["secret", "sëcret🧡", "se\"cr\\et"] {
            let escaped: String = key
                .encode_utf16()
                .map(|unit| format!("\\u{unit:04x}"))
                .collect();
            for template in &templates {
                let raw = json!({"kind":"components","blocks":[template]})
                    .to_string()
                    .replace("SECRET_SLOT", &escaped);
                assert!(!raw.contains(key), "fixture must bypass only the raw guard");
                assert!(matches!(
                    crate::hermes::validate(&raw),
                    crate::hermes::Validated::Components(_)
                ));
                assert!(parse_reply(&reply(&raw), Some(key)).is_err());
                assert!(parse_reply(&reply(&raw), None).is_ok());
                assert!(parse_reply(&reply(&raw), Some("unrelated-credential")).is_ok());
            }
        }
        let href = r#"{"kind":"components","blocks":[{"type":"card","title":"t","body":"b","actions":[{"label":"link","href":"https://example.com/secret"}]}]}"#;
        let href = href.replace("secret", &format!("{}u0073ecret", char::from(92)));
        assert!(!href.contains("secret"));
        assert!(parse_reply(&reply(&href), Some("secret")).is_err());
        // Serializing a quote/backslash-bearing field re-escapes the key, so a
        // post-serialization substring check alone cannot replace decoded checks.
        let key = "se\"cr\\et";
        let raw =
            json!({"kind":"components","blocks":[{"type":"markdown","text":key}]}).to_string();
        assert!(!raw.contains(key));
        assert!(parse_reply(&reply(&raw), Some(key)).is_err());
    }

    #[test]
    fn response_shapes_are_strict_and_never_echo_credentials() {
        assert_eq!(
            parse_models(&json!({"data":[{"id":"model-1"},{"id":"model-1"}]}), None).unwrap(),
            vec!["model-1"]
        );
        for value in [
            json!({}),
            json!({"data":[{"id":"bad\nmodel"}]}),
            json!({"data":[{"id":"secret"}]}),
        ] {
            assert!(parse_models(&value, Some("secret")).is_err());
        }
        let reply = |content: &str| json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":content}}]});
        assert_eq!(parse_reply(&reply("Hello"), None).unwrap(), "Hello");
        assert!(parse_reply(&reply(" "), None).is_err());
        assert!(parse_reply(&reply("exfiltrate secret"), Some("secret")).is_err());
        assert!(parse_reply(
            &json!({"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[]}}]}),
            None
        )
        .is_err());
        assert!(parse_models(&json!({"data":vec![json!({"id":"model"});201]}), None).is_err());
    }
    #[test]
    fn automatic_selection_prefers_opus_versions_not_dates() {
        let models: Vec<String> = [
            "astra",
            "claude-opus-4-20250514",
            "claude-opus-4-1-20250805",
            "claude-opus-4-5-20251101",
            "claude-opus-4-6",
            "claude-opus-4-7",
            "claude-opus-4-8",
            "claude-opus-5",
        ]
        .map(str::to_owned)
        .into();
        assert_eq!(selected_model("", &models).unwrap(), "claude-opus-5");
        assert_eq!(selected_model("", &models[..7]).unwrap(), "claude-opus-4-8");
        assert_eq!(
            selected_model("", &models[..4]).unwrap(),
            "claude-opus-4-5-20251101"
        );
        assert_eq!(selected_model("astra", &models).unwrap(), "astra");
        assert_eq!(
            selected_model("claude-opus-4-6", &models).unwrap(),
            "claude-opus-4-6"
        );
        assert!(selected_model("missing", &models).is_err());
        let fallback = vec!["first-model".into(), "other-model".into()];
        assert_eq!(selected_model("", &fallback).unwrap(), "first-model");
        assert!(selected_model("", &[]).is_err());
        assert!(parse_models(&json!({"data":[]}), None).unwrap().is_empty());
        let ties = vec!["claude-opus-5".into(), "claude-opus-5-20260101".into()];
        assert_eq!(selected_model("", &ties).unwrap(), "claude-opus-5");
        assert_eq!(opus_version("claude-opus-4-20250514"), Some((4, 0)));
        assert_eq!(opus_version("not-claude-opus-99"), None);
    }
    #[test]
    fn credential_names_are_endpoint_bound_and_qa_is_separate() {
        let reference = credential_reference("https://a.example/v1");
        assert!(credential_matches("https://a.example/v1", &reference));
        assert!(!credential_matches("https://b.example/v1", &reference));
        assert!(!credential_matches("https://a.example/v2", &reference));
        assert!(credential_reference("https://a.example/v1")
            .starts_with(&format!("{:x}", Sha256::digest(b"https://a.example/v1"))));
        assert_ne!(
            OsCredentials::new(None).service,
            OsCredentials::new(Some(Path::new("/tmp/qa"))).service
        );
        assert_ne!(
            OsCredentials::new(Some(Path::new("/tmp/qa1"))).service,
            OsCredentials::new(Some(Path::new("/tmp/qa2"))).service
        );
    }
}
