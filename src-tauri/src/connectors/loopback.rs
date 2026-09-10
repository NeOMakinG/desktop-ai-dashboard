//! One host-owned loopback callback. Unauthenticated probes never end a flow.
use crate::types::{AppError, AppResult};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

const MAX_REQUEST_BYTES: usize = 8_192;
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const IO_SLICE: Duration = Duration::from_millis(100);

pub struct Loopback {
    listener: TcpListener,
    port: u16,
}
pub struct Callback {
    pub code: Zeroizing<String>,
}

enum ParsedCallback {
    Ignore,
    Code(Zeroizing<String>),
    Denied,
}

impl Loopback {
    pub fn bind() -> AppResult<Self> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| bind_error())?;
        let port = listener.local_addr().map_err(|_| bind_error())?.port();
        listener.set_nonblocking(true).map_err(|_| bind_error())?;
        Ok(Self { listener, port })
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn redirect_uri(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    pub fn wait_for_callback(
        &self,
        state: &str,
        path: &str,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> AppResult<Callback> {
        loop {
            if cancel.is_cancelled() {
                return Err(super::lifecycle::stale_error());
            }
            if Instant::now() >= deadline {
                return Err(super::lifecycle::timeout_error());
            }
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    if peer.ip() != std::net::IpAddr::V4(Ipv4Addr::LOCALHOST) {
                        continue;
                    }
                    match handle_request(stream, self.port, state, path, deadline, cancel) {
                        ParsedCallback::Code(code) => return Ok(Callback { code }),
                        ParsedCallback::Denied => {
                            return Err(AppError::new(
                                "connector_denied",
                                "Google sign-in was declined. You can try again.",
                            ))
                        }
                        ParsedCallback::Ignore => {}
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(POLL_INTERVAL)
                }
                Err(_) => return Err(bind_error()),
            }
        }
    }
}

fn bind_error() -> AppError {
    AppError::new(
        "connector_loopback",
        "The temporary sign-in callback listener is unavailable.",
    )
}

/// Same strict identity predicate for browser admission and HTTP callback parsing.
/// URL query parsing decodes keys too, so encoded duplicate state cannot bypass it.
pub fn matches_callback_url(url: &reqwest::Url, port: u16, path: &str, state: &str) -> bool {
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port() != Some(port)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path() != path
        || url.as_str().len() > MAX_REQUEST_BYTES
    {
        return false;
    }
    let query = url.query().unwrap_or("");
    if !valid_encoding(query) {
        return false;
    }
    let states: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key == "state")
        .collect();
    states.len() == 1 && constant_time_eq(states[0].1.as_bytes(), state.as_bytes())
}

fn parse_target(target: &str, port: u16, state: &str, path: &str) -> ParsedCallback {
    // Only an origin-form target with the exact raw path is accepted.
    if target.split('?').next() != Some(path) || target.contains('#') {
        return ParsedCallback::Ignore;
    }
    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1:{port}{target}")) else {
        return ParsedCallback::Ignore;
    };
    if !matches_callback_url(&url, port, path, state) {
        return ParsedCallback::Ignore;
    }
    let mut code = None;
    let mut error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => {
                if code.is_some() {
                    return ParsedCallback::Ignore;
                }
                code = Some(Zeroizing::new(value.into_owned()));
            }
            "error" => {
                if error.is_some() {
                    return ParsedCallback::Ignore;
                }
                error = Some(value.into_owned());
            }
            _ => {}
        }
    }
    // State is already validated, including on the provider-denial path.
    match (code, error) {
        (None, Some(error)) if !error.is_empty() => ParsedCallback::Denied,
        (Some(code), None)
            if !code.is_empty()
                && code.len() <= 4096
                && code.bytes().all(|b| (33..=126).contains(&b)) =>
        {
            ParsedCallback::Code(code)
        }
        _ => ParsedCallback::Ignore,
    }
}

fn parse_request(request: &[u8], port: u16, state: &str, path: &str) -> ParsedCallback {
    if request.len() > MAX_REQUEST_BYTES {
        return ParsedCallback::Ignore;
    }
    let Ok(text) = std::str::from_utf8(request) else {
        return ParsedCallback::Ignore;
    };
    let Some((headers, _)) = text.split_once("\r\n\r\n") else {
        return ParsedCallback::Ignore;
    };
    let mut lines = headers.split("\r\n");
    let parts: Vec<_> = lines.next().unwrap_or("").split(' ').collect();
    if parts.len() != 3 || parts[0] != "GET" || !matches!(parts[2], "HTTP/1.1" | "HTTP/1.0") {
        return ParsedCallback::Ignore;
    }
    let expected_host = format!("127.0.0.1:{port}");
    let mut hosts = 0;
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            return ParsedCallback::Ignore;
        };
        if key.eq_ignore_ascii_case("host") {
            hosts += 1;
            if value.trim() != expected_host {
                return ParsedCallback::Ignore;
            }
        }
    }
    if hosts != 1 {
        return ParsedCallback::Ignore;
    }
    parse_target(parts[1], port, state, path)
}

fn handle_request(
    mut stream: TcpStream,
    port: u16,
    state: &str,
    path: &str,
    deadline: Instant,
    cancel: &CancellationToken,
) -> ParsedCallback {
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(IO_SLICE)).is_err()
        || stream.set_write_timeout(Some(IO_SLICE)).is_err()
    {
        return ParsedCallback::Ignore;
    }
    let read_deadline = deadline.min(Instant::now() + Duration::from_secs(1));
    let mut buffer = [0u8; 1024];
    let mut request = Zeroizing::new(Vec::with_capacity(1024));
    while Instant::now() < read_deadline && !cancel.is_cancelled() {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                request.extend_from_slice(&buffer[..n]);
                if request.len() > MAX_REQUEST_BYTES {
                    return ParsedCallback::Ignore;
                }
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    if Instant::now() >= deadline || cancel.is_cancelled() {
                        return ParsedCallback::Ignore;
                    }
                    let result = parse_request(&request, port, state, path);
                    let (status, body) = match &result {
                        ParsedCallback::Ignore => ("400 Bad Request", "Callback not accepted."),
                        ParsedCallback::Denied => ("200 OK", "Sign-in declined. Return to Forma."),
                        ParsedCallback::Code(_) => (
                            "200 OK",
                            "Callback received. Return to Forma to check sign-in status.",
                        ),
                    };
                    let _ = write_response(&mut stream, status, body);
                    return result;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => break,
        }
    }
    ParsedCallback::Ignore
}

fn write_response(stream: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\n\r\n{body}", body.len())
}

fn valid_encoding(input: &str) -> bool {
    let mut bytes = input.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%'
            && !(bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
                && bytes.next().is_some_and(|b| b.is_ascii_hexdigit()))
        {
            return false;
        }
    }
    true
}
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    const PATH: &str = "/oauth2/google/callback";
    #[test]
    fn unauthenticated_errors_and_malformed_probes_never_cancel() {
        for query in [
            "error=access_denied",
            "state=wrong&error=access_denied",
            "state=wrong&code=a",
            "state=ok&state=wrong&error=denied",
            "state=ok&%73tate=ok&error=denied",
            "state=ok&code=a&code=b",
            "state=ok&code=a&error=denied",
            "state=ok&error=denied&error=denied",
            "state=ok&code=%zz",
            "state=ok",
            "state=ok&code=%00",
            "state=ok&code=%C3%A9",
        ] {
            assert!(
                matches!(
                    parse_target(&format!("{PATH}?{query}"), 12345, "ok", PATH),
                    ParsedCallback::Ignore
                ),
                "query {query}"
            );
        }
        assert!(matches!(
            parse_target(
                &format!("{PATH}?state=ok&error=access_denied"),
                12345,
                "ok",
                PATH
            ),
            ParsedCallback::Denied
        ));
        assert!(
            matches!(parse_target(&format!("{PATH}?state=ok&code=a%2Fb"), 12345, "ok", PATH), ParsedCallback::Code(code) if *code == "a/b")
        );
    }
    #[test]
    fn callback_url_authority_is_exact() {
        let good = format!("http://127.0.0.1:12345{PATH}?state=ok&code=a");
        assert!(matches_callback_url(
            &reqwest::Url::parse(&good).unwrap(),
            12345,
            PATH,
            "ok"
        ));
        for bad in [
            good.replace("12345", "12346"),
            good.replace("127.0.0.1", "localhost"),
            good.replace("127.0.0.1", "[::1]"),
            good.replace("http:", "https:"),
            good.replace("127.0.0.1", "user@127.0.0.1"),
            good.replace("127.0.0.1", ":pass@127.0.0.1"),
            good.replace("callback?", "callback/other?"),
            format!("{good}#fragment"),
            format!("{good}&state=ok"),
            good.replace("state=ok", "state=wrong"),
        ] {
            assert!(
                !matches_callback_url(&reqwest::Url::parse(&bad).unwrap(), 12345, PATH, "ok"),
                "URL {bad}"
            );
        }
    }
    #[test]
    fn request_host_method_utf8_and_origin_form_are_strict() {
        let good = format!("GET {PATH}?state=ok&code=a HTTP/1.1\r\nHost: 127.0.0.1:12345\r\n\r\n");
        assert!(matches!(
            parse_request(good.as_bytes(), 12345, "ok", PATH),
            ParsedCallback::Code(_)
        ));
        for bad in [
            good.replace("GET ", "POST "),
            good.replace("Host:", "X-Host:"),
            good.replace(":12345", ":54321"),
            good.replace(PATH, &format!("http://127.0.0.1:12345{PATH}")),
            good.replace("\r\n\r\n", "\r\nHost: 127.0.0.1:12345\r\n\r\n"),
            good.replace("callback?", "other/../callback?"),
        ] {
            assert!(matches!(
                parse_request(bad.as_bytes(), 12345, "ok", PATH),
                ParsedCallback::Ignore
            ));
        }
        assert!(matches!(
            parse_request(&[0xff], 12345, "ok", PATH),
            ParsedCallback::Ignore
        ));
    }
    #[test]
    fn listener_is_loopback_and_cancel_timeout_drop_it_without_network() {
        let listener = Loopback::bind().unwrap();
        assert_eq!(
            listener.listener.local_addr().unwrap().ip(),
            std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        let cancel = CancellationToken::new();
        assert_eq!(
            listener
                .wait_for_callback("state", PATH, Instant::now(), &cancel)
                .err()
                .unwrap()
                .code,
            "connector_timeout"
        );
        cancel.cancel();
        assert_eq!(
            listener
                .wait_for_callback(
                    "state",
                    PATH,
                    Instant::now() + Duration::from_secs(1),
                    &cancel
                )
                .err()
                .unwrap()
                .code,
            "connector_stale"
        );
    }
}
