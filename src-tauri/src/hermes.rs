// Structured-component response over OpenAI-compat, not yet Hermes tools.
// This module is a pure host-side validator for the assistant JSON contract:
//   {"kind":"components","blocks":[ ...typed blocks... ]}
// It never touches the network, provider transport, or storage. Invalid input
// falls back to rendering the original model text as escaped plain text.

use serde::Serialize;
use serde_json::Value;
use zeroize::Zeroizing;

pub const MAX_BLOCKS: usize = 40;
pub const MAX_STRING: usize = 4_000;
pub const MAX_COMPONENT_BYTES: usize = 100_000;
pub const MAX_COMPONENT_DEPTH: usize = 8;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub enum Validated {
    Components(Vec<Block>),
    Fallback,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Block {
    Markdown {
        text: String,
    },
    Card {
        title: String,
        body: String,
        actions: Vec<Action>,
    },
    List {
        items: Vec<ListItem>,
    },
    Kv {
        rows: Vec<KvRow>,
    },
    Callout {
        tone: Tone,
        text: String,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Action {
    pub label: String,
    pub href: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct ListItem {
    pub title: String,
    pub detail: String,
    pub icon: Icon,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct KvRow {
    pub label: String,
    pub value: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Info,
    Warn,
    Success,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Icon {
    Envelope,
    Calendar,
    Globe,
    Sparkle,
}

/// Validate a raw assistant reply. Returns `Components` on a strictly-conforming
/// JSON envelope; anything else — non-JSON, wrong shape, unknown types, oversize
/// strings, http:// hrefs — returns `Fallback` so the renderer shows the raw text.
pub fn validate(raw: &str) -> Validated {
    match parse_envelope(raw) {
        Ok(blocks) => Validated::Components(blocks),
        Err(()) => Validated::Fallback,
    }
}

/// Canonicalize only a validated envelope at the provider boundary. Ordinary or
/// invalid content is untouched unless credential-bearing; no shape repair.
pub fn canonical_reply(raw: &str, forbidden: Option<&str>) -> Option<String> {
    if let Some(secret) = forbidden.filter(|secret| !secret.is_empty()) {
        // The provider enforces its stricter native text cap before this call.
        // Do not let schema failure or overwritten fields bypass this guard.
        if raw.len() > MAX_COMPONENT_BYTES || escaped_contains(raw, secret) {
            return None;
        }
    }
    match validate(raw) {
        Validated::Components(blocks) => {
            let value = serde_json::json!({ "kind": "components", "blocks": blocks });
            // Scan decoded fields before serialization: quotes/backslashes and
            // Unicode escapes can hide a credential from raw JSON substring checks.
            if forbidden
                .is_some_and(|secret| !secret.is_empty() && decoded_contains(&value, secret))
            {
                return None;
            }
            let canonical =
                serde_json::to_string(&value).expect("validated strings are JSON serializable");
            // Normalizing omitted actions can grow a boundary-size envelope.
            Some(if canonical.len() <= MAX_COMPONENT_BYTES {
                canonical
            } else {
                raw.to_owned()
            })
        }
        Validated::Fallback => Some(raw.to_owned()),
    }
}

// One JSON-escape layer only, not general encoded-secret DLP. Scanning the whole
// bounded input also covers unclosed strings, trailing content, overwritten keys
// and escaped prose without trusting JSON structure or parsing numeric tokens.
// Invalid escapes remain literal; delimiters stay present, never joining fields.
pub(crate) fn escaped_contains(raw: &str, needle: &str) -> bool {
    let mut decoded = Zeroizing::new(String::with_capacity(raw.len()));
    let mut index = 0;
    while index < raw.len() {
        if let Some((ch, consumed)) = json_escape(&raw.as_bytes()[index..]) {
            decoded.push(ch);
            index += consumed;
        } else {
            let ch = raw[index..]
                .chars()
                .next()
                .expect("index is a UTF-8 boundary");
            decoded.push(ch);
            index += ch.len_utf8();
        }
    }
    decoded.contains(needle)
}

// Decode a single escape with at most 12 bytes of lookahead. Unpaired UTF-16
// surrogates are not Unicode scalars and must not become replacement characters.
fn json_escape(bytes: &[u8]) -> Option<(char, usize)> {
    if bytes.first() != Some(&b'\\') {
        return None;
    }
    let ch = match bytes.get(1)? {
        b'"' => '"',
        b'\\' => '\\',
        b'/' => '/',
        b'b' => '\u{0008}',
        b'f' => '\u{000c}',
        b'n' => '\n',
        b'r' => '\r',
        b't' => '\t',
        b'u' => {
            let high = hex_quad(bytes.get(2..6)?)?;
            if (0xd800..=0xdbff).contains(&high) {
                if bytes.get(6..8)? != b"\\u" {
                    return None;
                }
                let low = hex_quad(bytes.get(8..12)?)?;
                if !(0xdc00..=0xdfff).contains(&low) {
                    return None;
                }
                let scalar = 0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00);
                return char::from_u32(scalar).map(|ch| (ch, 12));
            }
            return char::from_u32(high).map(|ch| (ch, 6));
        }
        _ => return None,
    };
    Some((ch, 2))
}

fn hex_quad(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0, |value, byte| {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        Some(value * 16 + u32::from(digit))
    })
}

// This value comes only from the validated, fixed-depth typed block tree.
fn decoded_contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Array(items) => items.iter().any(|item| decoded_contains(item, needle)),
        Value::Object(fields) => fields
            .iter()
            .any(|(key, item)| key.contains(needle) || decoded_contains(item, needle)),
        _ => false,
    }
}

fn bounded_json(raw: &str) -> Result<(), ()> {
    if raw.len() > MAX_COMPONENT_BYTES {
        return Err(());
    }
    let mut depth: usize = 0;
    let mut in_string = false;
    let mut escaped = false;
    for byte in raw.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                // No numeric fields exist; reject even overwritten numbers to
                // avoid JS/serde_json float-overflow rounding disagreements.
                b'0'..=b'9' => return Err(()),
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > MAX_COMPONENT_DEPTH {
                        return Err(());
                    }
                }
                b'}' | b']' => depth = depth.checked_sub(1).ok_or(())?,
                _ => {}
            }
        }
    }
    Ok(())
}

fn parse_envelope(raw: &str) -> Result<Vec<Block>, ()> {
    bounded_json(raw)?;
    // serde_json handles JSON whitespace, not the larger Unicode trim set.
    let value: Value = serde_json::from_str(raw).map_err(|_| ())?;
    let object = value.as_object().ok_or(())?;
    // Exactly two keys: "kind" and "blocks" — nothing else may ride along.
    if object.len() != 2 {
        return Err(());
    }
    if object.get("kind").and_then(Value::as_str) != Some("components") {
        return Err(());
    }
    let blocks = object.get("blocks").and_then(Value::as_array).ok_or(())?;
    if blocks.len() > MAX_BLOCKS {
        return Err(());
    }
    blocks.iter().map(parse_block).collect()
}

fn parse_block(value: &Value) -> Result<Block, ()> {
    let object = value.as_object().ok_or(())?;
    let ty = object.get("type").and_then(Value::as_str).ok_or(())?;
    match ty {
        "markdown" => {
            if object.len() != 2 {
                return Err(());
            }
            Ok(Block::Markdown {
                text: string(object.get("text"))?,
            })
        }
        "card" => {
            let title = string(object.get("title"))?;
            let body = string(object.get("body"))?;
            let actions = match object.get("actions") {
                Some(Value::Array(items)) => {
                    if items.len() > MAX_BLOCKS {
                        return Err(());
                    }
                    items.iter().map(parse_action).collect::<Result<_, _>>()?
                }
                Some(_) => return Err(()),
                None => Vec::new(),
            };
            let expected = 3 + usize::from(object.contains_key("actions"));
            if object.len() != expected {
                return Err(());
            }
            Ok(Block::Card {
                title,
                body,
                actions,
            })
        }
        "list" => {
            if object.len() != 2 {
                return Err(());
            }
            let items = object.get("items").and_then(Value::as_array).ok_or(())?;
            if items.len() > MAX_BLOCKS {
                return Err(());
            }
            let items = items
                .iter()
                .map(parse_list_item)
                .collect::<Result<_, _>>()?;
            Ok(Block::List { items })
        }
        "kv" => {
            if object.len() != 2 {
                return Err(());
            }
            let rows = object.get("rows").and_then(Value::as_array).ok_or(())?;
            if rows.len() > MAX_BLOCKS {
                return Err(());
            }
            let rows = rows.iter().map(parse_row).collect::<Result<_, _>>()?;
            Ok(Block::Kv { rows })
        }
        "callout" => {
            if object.len() != 3 {
                return Err(());
            }
            let tone = match object.get("tone").and_then(Value::as_str).ok_or(())? {
                "info" => Tone::Info,
                "warn" => Tone::Warn,
                "success" => Tone::Success,
                _ => return Err(()),
            };
            Ok(Block::Callout {
                tone,
                text: string(object.get("text"))?,
            })
        }
        _ => Err(()),
    }
}

fn parse_action(value: &Value) -> Result<Action, ()> {
    let object = value.as_object().ok_or(())?;
    if object.len() != 2 {
        return Err(());
    }
    let label = string(object.get("label"))?;
    let href = string(object.get("href"))?;
    if !valid_href(&href) {
        return Err(());
    }
    Ok(Action { label, href })
}

// Same deliberately conservative HTTPS subset as message-contracts.ts. No URL
// validation result grants navigation, account access, or a native capability.
fn valid_href(href: &str) -> bool {
    let Some(rest) = href.strip_prefix("https://") else {
        return false;
    };
    if href.chars().any(|ch| {
        matches!(ch as u32,
            0..=0x20 | 0x7f..=0xa0 | 0x1680 | 0x2000..=0x200f |
            0x2028..=0x202f | 0x205f..=0x206f | 0x3000 | 0xfeff | 0x5c
        )
    }) {
        return false;
    }
    let bytes = href.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(pair) = bytes.get(index + 1..index + 3) else {
                return false;
            };
            if !pair.iter().all(u8::is_ascii_hexdigit) {
                return false;
            }
            let byte = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
            if byte <= 0x20 || byte == 0x7f || byte == 0x5c {
                return false;
            }
            index += 2;
        }
        index += 1;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty()
        || !authority
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'-'))
    {
        return false;
    }
    let mut parts = authority.split(':');
    let host = parts.next().unwrap_or("");
    if let Some(port) = parts.next() {
        if port.is_empty()
            || port.len() > 5
            || !port.bytes().all(|b| b.is_ascii_digit())
            || !port.parse::<u16>().is_ok_and(|p| p > 0)
        {
            return false;
        }
    }
    if parts.next().is_some() || host.len() > 253 {
        return false;
    }
    let labels: Vec<_> = host.split('.').collect();
    if labels.iter().any(|label| {
        label.is_empty()
            || label.len() > 63
            || !label.as_bytes()[0].is_ascii_alphanumeric()
            || !label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }) {
        return false;
    }
    let last = labels.last().unwrap().to_ascii_lowercase();
    if last.bytes().all(|b| b.is_ascii_digit())
        || last
            .strip_prefix("0x")
            .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        if labels.len() != 4
            || labels.iter().any(|label| {
                (label.len() > 1 && label.starts_with('0'))
                    || !label.bytes().all(|b| b.is_ascii_digit())
                    || label.parse::<u8>().is_err()
            })
        {
            return false;
        }
    }
    reqwest::Url::parse(href).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn parse_list_item(value: &Value) -> Result<ListItem, ()> {
    let object = value.as_object().ok_or(())?;
    if object.len() != 3 {
        return Err(());
    }
    let title = string(object.get("title"))?;
    let detail = string(object.get("detail"))?;
    let icon = match object.get("icon").and_then(Value::as_str).ok_or(())? {
        "envelope" => Icon::Envelope,
        "calendar" => Icon::Calendar,
        "globe" => Icon::Globe,
        "sparkle" => Icon::Sparkle,
        _ => return Err(()),
    };
    Ok(ListItem {
        title,
        detail,
        icon,
    })
}

fn parse_row(value: &Value) -> Result<KvRow, ()> {
    let object = value.as_object().ok_or(())?;
    if object.len() != 2 {
        return Err(());
    }
    let label = string(object.get("label"))?;
    let value = string(object.get("value"))?;
    Ok(KvRow { label, value })
}

fn string(value: Option<&Value>) -> Result<String, ()> {
    let text = value.and_then(Value::as_str).ok_or(())?;
    // UTF-8 bytes, not UTF-16 units. Oversized fields fall back; never truncate.
    if text.len() > MAX_STRING || text.contains('\0') {
        return Err(());
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope(blocks: serde_json::Value) -> String {
        json!({"kind": "components", "blocks": blocks}).to_string()
    }

    #[test]
    fn shared_component_conformance_fixtures() {
        let fixtures: Vec<Value> =
            serde_json::from_str(include_str!("../../src/app/message-conformance.json")).unwrap();
        for fixture in fixtures {
            let mut input = if let Some(href) = fixture.get("href") {
                json!({"kind":"components","blocks":[{"type":"card","title":"t","body":"b","actions":[{"label":"link","href":href}]}]})
            } else {
                fixture["input"].clone()
            };
            if let Some(repeat) = fixture.get("repeat") {
                let count = repeat["count"].as_u64().unwrap() as usize;
                let unit = &repeat["unit"];
                let value = match unit.as_str() {
                    Some(text) => json!(text.repeat(count)),
                    None => json!(vec![unit.clone(); count]),
                };
                *input.pointer_mut(repeat["path"].as_str().unwrap()).unwrap() = value;
            }
            let mut raw = match input.as_str() {
                Some(raw) => raw.to_owned(),
                None => input.to_string(),
            };
            if let Some(length) = fixture.get("padTo") {
                raw.push_str(
                    &" ".repeat((length.as_u64().unwrap() as usize).saturating_sub(raw.len())),
                );
            }
            let valid = matches!(validate(&raw), Validated::Components(_));
            assert_eq!(
                valid,
                fixture["valid"].as_bool().unwrap(),
                "fixture: {}",
                fixture["name"]
            );
            let canonical = canonical_reply(&raw, None).unwrap();
            if valid {
                assert_eq!(
                    validate(&canonical),
                    validate(&raw),
                    "roundtrip: {}",
                    fixture["name"]
                );
            } else {
                assert_eq!(canonical, raw, "fallback changed: {}", fixture["name"]);
            }
        }
    }

    #[test]
    fn credential_scan_decodes_one_json_escape_layer_without_schema_parsing() {
        for (raw, decoded) in [
            (r#"\"\\\/\b\f\n\r\t"#, "\"\\/\u{0008}\u{000c}\n\r\t"),
            (r#"\uD800secret"#, r#"\uD800secret"#),
            (r#"\uDDE1secret"#, r#"\uDDE1secret"#),
            (r#"\uNOPEsecret"#, r#"\uNOPEsecret"#),
            (r#"\qsecret\"#, "\\qsecret\\"),
        ] {
            assert!(escaped_contains(raw, decoded), "input: {raw:?}");
        }
        let scalars = "\0aé🧡";
        let escaped: String = scalars
            .encode_utf16()
            .map(|unit| format!("\\u{unit:04x}"))
            .collect();
        assert!(escaped_contains(&escaped, scalars));
        assert!(!escaped_contains(r#"\uD800"#, "�"));
        assert!(!escaped_contains(r#"\uDDE1"#, "�"));
        // No recursive decoding, concatenation across JSON fields, or removal
        // of malformed escapes: this is intentionally not general secret DLP.
        assert!(escaped_contains(r#"\\u0073ecret"#, "\\u0073ecret"));
        assert!(!escaped_contains(r#"\\u0073ecret"#, "secret"));
        assert!(!escaped_contains(r#"["sec","ret"]"#, "secret"));
        assert!(!escaped_contains(r#"sec\qret"#, "secret"));
    }

    #[test]
    fn credential_scan_covers_size_fallbacks_and_is_bounded() {
        let escaped = "\\u0073ecret";
        let large_field = envelope(json!([{"type":"markdown","text":"x".repeat(MAX_STRING + 1)}]));
        let too_many_blocks = envelope(json!(vec![
            json!({"type":"markdown","text":"x"});
            MAX_BLOCKS + 1
        ]));
        for raw in [
            large_field.replace("xxxx", escaped),
            too_many_blocks.replace("\"x\"", &format!("\"{escaped}\"")),
            format!(
                "{}{escaped}",
                "x".repeat(MAX_COMPONENT_BYTES - escaped.len())
            ),
        ] {
            assert_eq!(validate(&raw), Validated::Fallback);
            assert_eq!(canonical_reply(&raw, Some("secret")), None);
            assert_eq!(
                canonical_reply(&raw, Some("unrelated-credential")),
                Some(raw)
            );
        }
        let over_cap = "x".repeat(MAX_COMPONENT_BYTES + 1);
        assert_eq!(canonical_reply(&over_cap, Some("secret")), None);
        // Unauthenticated validator callers retain the existing raw fallback.
        assert_eq!(canonical_reply(&over_cap, None), Some(over_cap));
    }

    #[test]
    fn valid_envelope_yields_components_for_every_block_type() {
        let raw = envelope(json!([
            {"type": "markdown", "text": "Hello"},
            {"type": "card", "title": "T", "body": "B", "actions": [
                {"label": "Open", "href": "https://example.com"}
            ]},
            {"type": "card", "title": "No actions", "body": "B"},
            {"type": "list", "items": [
                {"title": "One", "detail": "d", "icon": "envelope"},
                {"title": "Two", "detail": "d", "icon": "sparkle"}
            ]},
            {"type": "kv", "rows": [{"label": "L", "value": "V"}]},
            {"type": "callout", "tone": "info", "text": "note"},
            {"type": "callout", "tone": "warn", "text": "note"},
            {"type": "callout", "tone": "success", "text": "note"}
        ]));
        let Validated::Components(blocks) = validate(&raw) else {
            panic!("expected Components");
        };
        assert_eq!(blocks.len(), 8);
        assert!(matches!(blocks[0], Block::Markdown { .. }));
        assert!(matches!(
            blocks[7],
            Block::Callout {
                tone: Tone::Success,
                ..
            }
        ));
    }

    #[test]
    fn non_json_and_shape_violations_fall_back() {
        for raw in [
            "",
            "   ",
            "not json at all",
            "Here is a thought.\nAnd a follow-up.",
            "[1, 2, 3]",
            "\"just a string\"",
            "{}",
            "{\"kind\":\"components\"}",
            "{\"kind\":\"other\",\"blocks\":[]}",
            "{\"kind\":\"components\",\"blocks\":[],\"extra\":1}",
            "{\"kind\":\"components\",\"blocks\":{}}",
        ] {
            assert_eq!(validate(raw), Validated::Fallback, "input: {raw:?}");
        }
    }

    #[test]
    fn unknown_types_and_unknown_enums_are_rejected() {
        for blocks in [
            json!([{"type": "iframe", "src": "https://x"}]),
            json!([{"type": "markdown"}]),
            json!([{"type": "markdown", "text": "ok", "extra": 1}]),
            json!([{"type": "callout", "tone": "danger", "text": "x"}]),
            json!([{"type": "list", "items": [{"title": "t", "detail": "d", "icon": "rocket"}]}]),
            json!([{"type": "card", "title": "t", "body": "b", "actions": "nope"}]),
        ] {
            let raw = envelope(blocks);
            assert_eq!(validate(&raw), Validated::Fallback, "input: {raw}");
        }
    }

    #[test]
    fn hrefs_must_be_https_and_non_empty() {
        for href in [
            "http://example.com",
            "javascript:alert(1)",
            "//example.com",
            "https://",
            "  https://example.com",
        ] {
            let raw = envelope(json!([{
                "type": "card", "title": "t", "body": "b",
                "actions": [{"label": "go", "href": href}]
            }]));
            assert_eq!(validate(&raw), Validated::Fallback, "href: {href}");
        }
        let good = envelope(json!([{
            "type": "card", "title": "t", "body": "b",
            "actions": [{"label": "go", "href": "https://example.com/path"}]
        }]));
        assert!(matches!(validate(&good), Validated::Components(_)));
    }

    #[test]
    fn oversize_arrays_and_strings_are_rejected() {
        let too_many_blocks: Vec<_> = (0..=MAX_BLOCKS)
            .map(|_| json!({"type": "markdown", "text": "x"}))
            .collect();
        assert_eq!(
            validate(&envelope(json!(too_many_blocks))),
            Validated::Fallback
        );
        let too_many_items: Vec<_> = (0..=MAX_BLOCKS)
            .map(|_| json!({"title": "t", "detail": "d", "icon": "globe"}))
            .collect();
        assert_eq!(
            validate(&envelope(
                json!([{"type": "list", "items": too_many_items}])
            )),
            Validated::Fallback
        );
        let long = "x".repeat(MAX_STRING + 1);
        assert_eq!(
            validate(&envelope(json!([{"type": "markdown", "text": long}]))),
            Validated::Fallback
        );
        let with_null = "abc\0def".to_owned();
        assert_eq!(
            validate(&envelope(json!([{"type": "markdown", "text": with_null}]))),
            Validated::Fallback
        );
        // Exactly at the boundary is still valid — the limit is inclusive.
        let boundary = "x".repeat(MAX_STRING);
        assert!(matches!(
            validate(&envelope(json!([{"type": "markdown", "text": boundary}]))),
            Validated::Components(_)
        ));
    }

    #[test]
    fn fallback_path_exists_for_plain_prose_and_unwrapped_markdown() {
        // Concrete assertion that the Fallback branch is reachable — the renderer
        // relies on this to keep working when the model returns plain text.
        let cases = [
            "I can't do that yet — no connectors are enabled.",
            "```json\n{\"kind\":\"components\",\"blocks\":[]}\n```",
            "Prefixed prose {\"kind\":\"components\",\"blocks\":[]}",
        ];
        for raw in cases {
            assert!(matches!(validate(raw), Validated::Fallback), "input: {raw}");
        }
    }
}
