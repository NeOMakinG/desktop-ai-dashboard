//! Google OAuth 2.0 PKCE (installed-app) helpers. No secrets are logged.
use crate::types::{AppError, AppResult};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

pub const AUTHORIZE_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
pub const MAX_TOKEN_BYTES: usize = 8_192;
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_EXPIRES_IN: u64 = 60 * 60 * 24 * 30;

pub struct PkcePair {
    pub verifier: Zeroizing<String>,
    pub challenge: String,
}

pub fn random_bytes(n: usize) -> Vec<u8> {
    // Uuid v4 uses OS randomness. Concatenate to reach the desired length.
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        out.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    out.truncate(n);
    out
}

pub fn base64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let b0 = bytes[i];
        let b1 = bytes[i + 1];
        let b2 = bytes[i + 2];
        out.push(A[(b0 >> 2) as usize] as char);
        out.push(A[(((b0 & 0b11) << 4) | (b1 >> 4)) as usize] as char);
        out.push(A[(((b1 & 0b1111) << 2) | (b2 >> 6)) as usize] as char);
        out.push(A[(b2 & 0b111111) as usize] as char);
        i += 3;
    }
    match bytes.len() - i {
        1 => {
            let b = bytes[i];
            out.push(A[(b >> 2) as usize] as char);
            out.push(A[((b & 0b11) << 4) as usize] as char);
        }
        2 => {
            let b0 = bytes[i];
            let b1 = bytes[i + 1];
            out.push(A[(b0 >> 2) as usize] as char);
            out.push(A[(((b0 & 0b11) << 4) | (b1 >> 4)) as usize] as char);
            out.push(A[((b1 & 0b1111) << 2) as usize] as char);
        }
        _ => {}
    }
    out
}

pub fn pkce_pair() -> PkcePair {
    // 32 random bytes yields a 43-char base64url string, within PKCE's 43..=128 window.
    let verifier = base64url(&random_bytes(32));
    let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
    PkcePair {
        verifier: Zeroizing::new(verifier),
        challenge,
    }
}

pub fn generate_state() -> String {
    base64url(&random_bytes(32))
}

fn percent_encode(input: &str, out: &mut String) {
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            let hi = b >> 4;
            let lo = b & 0xF;
            out.push(char::from(if hi < 10 { b'0' + hi } else { b'A' + hi - 10 }));
            out.push(char::from(if lo < 10 { b'0' + lo } else { b'A' + lo - 10 }));
        }
    }
}

pub fn authorize_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    state: &str,
    challenge: &str,
) -> String {
    let scope = scopes.join(" ");
    let pairs: [(&str, &str); 9] = [
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("scope", scope.as_str()),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    let mut out = String::from(AUTHORIZE_ENDPOINT);
    out.push('?');
    let mut first = true;
    for (k, v) in pairs {
        if !first {
            out.push('&');
        }
        first = false;
        out.push_str(k);
        out.push('=');
        percent_encode(v, &mut out);
    }
    out
}

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
}

impl Drop for TokenResponse {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

fn response_error() -> AppError {
    AppError::new(
        "connector_response",
        "The provider returned an unsupported token response.",
    )
}

pub fn parse_token_response(bytes: &[u8]) -> AppResult<TokenResponse> {
    if bytes.is_empty() || bytes.len() > MAX_RESPONSE_BYTES {
        return Err(response_error());
    }
    let value: TokenResponse = serde_json::from_slice(bytes).map_err(|_| response_error())?;
    if value.access_token.is_empty() || value.access_token.len() > MAX_TOKEN_BYTES {
        return Err(response_error());
    }
    if !value.access_token.bytes().all(|b| (32..=126).contains(&b)) {
        return Err(response_error());
    }
    if let Some(tt) = value.token_type.as_deref() {
        if !tt.eq_ignore_ascii_case("bearer") {
            return Err(response_error());
        }
    }
    if let Some(rt) = value.refresh_token.as_deref() {
        if rt.is_empty()
            || rt.len() > MAX_TOKEN_BYTES
            || !rt.bytes().all(|b| (32..=126).contains(&b))
        {
            return Err(response_error());
        }
    }
    if let Some(exp) = value.expires_in {
        if exp == 0 || exp > MAX_EXPIRES_IN {
            return Err(response_error());
        }
    }
    Ok(value)
}

pub fn expires_at_from_secs(now: DateTime<Utc>, secs: u64) -> String {
    let bounded = secs.min(MAX_EXPIRES_IN);
    (now + ChronoDuration::seconds(bounded as i64)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_is_padless_and_matches_known_vectors() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foob"), "Zm9vYg");
        assert_eq!(base64url(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64url(&[0xff, 0xff, 0xff]), "____");
    }

    #[test]
    fn pkce_pair_meets_rfc7636_shape() {
        let pair = pkce_pair();
        assert!(pair.verifier.len() >= 43 && pair.verifier.len() <= 128);
        for b in pair.verifier.bytes() {
            assert!(b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'));
        }
        assert_eq!(
            pair.challenge,
            base64url(&Sha256::digest(pair.verifier.as_bytes()))
        );
        assert_eq!(pair.challenge.len(), 43);
        let another = pkce_pair();
        assert_ne!(pair.verifier.as_str(), another.verifier.as_str());
        assert_ne!(pair.challenge, another.challenge);
    }

    #[test]
    fn state_is_random_and_unique() {
        let a = generate_state();
        let b = generate_state();
        assert!(a.len() >= 32);
        assert_ne!(a, b);
    }

    #[test]
    fn authorize_url_encodes_every_required_pkce_parameter() {
        let scopes = vec![
            "https://www.googleapis.com/auth/gmail.metadata".to_owned(),
            "https://www.googleapis.com/auth/gmail.readonly".to_owned(),
            "https://www.googleapis.com/auth/calendar.readonly".to_owned(),
        ];
        let url = authorize_url(
            "client.example",
            "http://127.0.0.1:5555/oauth2/google/callback",
            &scopes,
            "state-abc",
            "challenge-abc",
        );
        assert!(url.starts_with(AUTHORIZE_ENDPOINT));
        for expected in [
            "response_type=code",
            "client_id=client.example",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A5555%2Foauth2%2Fgoogle%2Fcallback",
            "scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.metadata%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.readonly%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcalendar.readonly",
            "state=state-abc",
            "code_challenge=challenge-abc",
            "code_challenge_method=S256",
            "access_type=offline",
            "prompt=consent",
        ] {
            assert!(url.contains(expected), "missing `{expected}` in `{url}`");
        }
        assert!(!url.contains(' '));
        assert!(!url.contains('#'));
    }

    #[test]
    fn parse_token_response_rejects_bad_shapes_and_secrets_shaped_wrong() {
        let ok = br#"{"access_token":"aaa","refresh_token":"rrr","expires_in":3600,"token_type":"Bearer","scope":"s"}"#;
        let parsed = parse_token_response(ok).unwrap();
        assert_eq!(parsed.access_token, "aaa");
        assert_eq!(parsed.refresh_token.as_deref(), Some("rrr"));
        assert_eq!(parsed.expires_in, Some(3600));

        assert!(parse_token_response(b"").is_err());
        assert!(parse_token_response(b"not-json").is_err());
        assert!(parse_token_response(br#"{"access_token":""}"#).is_err());
        assert!(parse_token_response(br#"{"access_token":"abc","token_type":"MAC"}"#).is_err());
        assert!(parse_token_response(br#"{"access_token":"abc","expires_in":0}"#).is_err());
        assert!(
            parse_token_response(br#"{"access_token":"abc","expires_in":99999999999}"#).is_err()
        );
        assert!(parse_token_response(br#"{"access_token":"abc","refresh_token":""}"#).is_err());
        let huge = format!(
            "{{\"access_token\":\"{}\"}}",
            "a".repeat(MAX_TOKEN_BYTES + 1)
        );
        assert!(parse_token_response(huge.as_bytes()).is_err());
        assert!(parse_token_response("{\"access_token\":\"with\nnewline\"}".as_bytes()).is_err());
    }

    #[test]
    fn expires_at_math_is_bounded_and_iso_formatted() {
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(expires_at_from_secs(now, 0), "2026-01-01T00:00:00.000Z");
        assert_eq!(expires_at_from_secs(now, 3600), "2026-01-01T01:00:00.000Z");
        let capped = expires_at_from_secs(now, u64::MAX);
        assert!(capped.starts_with("2026-01-31T00:00:00"));
    }
}
