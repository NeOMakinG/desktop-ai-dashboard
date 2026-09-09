use crate::types::{AppError, AppResult};
use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr};

pub const MAX_TEXT: usize = 32 * 1024;
pub const MAX_CONTEXT: usize = 128 * 1024;
pub const MAX_RESPONSE: usize = 512 * 1024;
pub const MAX_VERSION: u64 = 9_007_199_254_740_990;

pub fn text(value: &str, max: usize, empty: bool) -> AppResult<()> {
    if value.len() > max || (!empty && value.trim().is_empty()) || value.contains('\0') {
        return Err(AppError::invalid());
    }
    Ok(())
}
pub fn single_line(value: &str, max: usize, empty: bool) -> AppResult<()> {
    text(value, max, empty)?;
    if value.chars().any(char::is_control) {
        return Err(AppError::invalid());
    }
    Ok(())
}
pub fn model(value: &str, empty: bool) -> AppResult<()> {
    single_line(value, 200, empty)?;
    if value.chars().any(char::is_whitespace) {
        return Err(AppError::invalid());
    }
    Ok(())
}
pub fn id(value: &str) -> AppResult<()> {
    let parsed = uuid::Uuid::parse_str(value).map_err(|_| AppError::invalid())?;
    if parsed.to_string() != value || parsed.is_nil() {
        return Err(AppError::invalid());
    }
    Ok(())
}
pub fn api_key(value: &str) -> AppResult<()> {
    if value.is_empty() || value.len() > 4096 || !value.bytes().all(|c| (33..=126).contains(&c)) {
        return Err(AppError::invalid());
    }
    Ok(())
}
fn private_http(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    if host == "localhost" || host.ends_with(".ts.net") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_loopback()
                || (u32::from(ip) & 0xffc0_0000 == u32::from(Ipv4Addr::new(100, 64, 0, 0)))
        }
        Ok(IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => false,
    }
}
pub fn endpoint(value: &str) -> AppResult<String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    single_line(value, 2048, false)?;
    // Reject URL-parser normalization of credentials, escapes, dot segments and whitespace.
    if value.contains(['\\', '%', '?', '#']) || value.chars().any(char::is_whitespace) {
        return Err(AppError::invalid());
    }
    let url = Url::parse(value).map_err(|_| AppError::invalid())?;
    let host = url.host_str().ok_or_else(AppError::invalid)?;
    if !url.username().is_empty()
        || url.password().is_some()
        || value.contains('@')
        || !matches!(url.scheme(), "http" | "https")
        || (url.scheme() == "http" && !private_http(host))
    {
        return Err(AppError::invalid());
    }
    let raw_path = value
        .split_once("://")
        .ok_or_else(AppError::invalid)?
        .1
        .split_once('/')
        .map(|(_, p)| p)
        .unwrap_or("");
    if raw_path.split('/').any(|part| part == "." || part == "..")
        || raw_path.contains("//")
        || raw_path.starts_with('/')
        || !raw_path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/-_.~".contains(&b))
    {
        return Err(AppError::invalid());
    }
    // HTTPS does not make metadata/link-local/unspecified targets suitable providers.
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        if !allowed_address(ip) {
            return Err(AppError::invalid());
        }
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub fn allowed_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_link_local()
                && ip.octets()[0] != 0
                && !ip.is_multicast()
                && ip != Ipv4Addr::BROADCAST
        }
        IpAddr::V6(ip) => {
            !ip.is_unspecified()
                && !ip.is_multicast()
                && (ip.segments()[0] & 0xffc0 != 0xfe80)
                && ip.to_ipv4_mapped().is_none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_policy_and_normalization() {
        for value in [
            "https://example.com/v1/",
            "http://localhost:9220/v1",
            "http://127.0.0.1:9220",
            "http://[::1]:9220",
            "http://100.64.0.10:9220",
            "http://workstation.example.ts.net:9220",
        ] {
            assert!(endpoint(value).is_ok(), "{value}");
        }
        assert_eq!(
            endpoint("https://EXAMPLE.com/v1/").unwrap(),
            "https://example.com/v1"
        );
        for value in [
            "http://example.com",
            "http://192.168.1.1",
            "http://169.254.169.254",
            "https://169.254.169.254",
            "https://user:key@example.com",
            "https://example.com?key=x",
            "https://example.com/#x",
            "file:///tmp/a",
            "https://example.com/a/../v1",
            "https://example.com/%2e",
            "https://example.com//v1",
            "https://example.com\\v1",
            "https://[::ffff:169.254.169.254]",
            "http://100.128.0.1",
            "http://evilts.net",
        ] {
            assert!(endpoint(value).is_err(), "{value}");
        }
    }
    #[test]
    fn resolved_metadata_and_ambiguous_addresses_are_rejected() {
        for address in [
            "169.254.169.254",
            "fe80::1",
            "::ffff:169.254.169.254",
            "0.0.0.0",
            "0.1.2.3",
            "224.0.0.1",
            "ff02::1",
        ] {
            assert!(!allowed_address(address.parse().unwrap()), "{address}");
        }
        for address in ["127.0.0.1", "::1", "100.64.0.10", "1.1.1.1"] {
            assert!(allowed_address(address.parse().unwrap()), "{address}");
        }
    }
    #[test]
    fn input_limits_and_ids() {
        assert!(text(&"a".repeat(MAX_TEXT + 1), MAX_TEXT, true).is_err());
        assert!(single_line("title\nspoof", 100, false).is_err());
        assert!(model("model name", false).is_err());
        assert!(api_key("key\r\nAuthorization: x").is_err());
        assert!(id("not-a-uuid").is_err());
        assert!(id(&uuid::Uuid::new_v4().to_string()).is_ok());
    }
}
