//! Minimal HTTP/1.1 request parsing, routing, and response building for the
//! on-device web server.
//!
//! This is the *pure* half of the web server: given the raw bytes of an
//! accumulated HTTP request, it parses the method/path/body, decides which
//! route matches, validates a `POST /config` body, and builds response
//! headers. The firmware owns the TCP socket and the `await`s; this module
//! does no I/O, so it is fully unit-testable on the host.
//!
//! Endpoints served:
//! - `GET /` and `GET /index.html` → the embedded UI page.
//! - `GET /config` → the current effect config JSON.
//! - `POST /config` → replace the effect config (validated here).

use alloc::string::String;

use crate::config::parse_config;

/// A parsed HTTP request (headers + body already accumulated by the caller).
#[derive(Debug)]
pub struct ParsedRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    /// The declared `Content-Length` (0 if absent).
    pub content_length: usize,
    /// The raw request body (only as many bytes as were actually received).
    pub body: &'a [u8],
}

/// The route a (method, path) maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Serve the embedded UI page.
    Index,
    /// Serve the current config JSON.
    GetConfig,
    /// Replace the config.
    PostConfig,
    /// Nothing matches.
    NotFound,
}

/// The outcome of validating a `POST /config` body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigPostResult {
    /// The body is valid JSON with ≥1 effect and fits in `max`.
    Accepted,
    /// The body is too large to store.
    TooLarge,
    /// The body is valid JSON but has no effects.
    NoEffects,
    /// The body is not valid config JSON.
    BadJson,
}

/// Write `n` as decimal ASCII into `buf`; returns the number of bytes written.
///
/// `buf` must be at least [`UINT_BUF_BYTES`] long.
pub const UINT_BUF_BYTES: usize = 20; // enough for a 64-bit usize

pub fn write_uint(buf: &mut [u8], n: usize) -> usize {
    let mut digits = [0u8; UINT_BUF_BYTES];
    let mut i = digits.len();
    let mut v = n;
    if v == 0 {
        buf[0] = b'0';
        return 1;
    }
    while v > 0 {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let len = digits.len() - i;
    buf[..len].copy_from_slice(&digits[i..]);
    len
}

/// Index of the `"\r\n\r\n"` header terminator, if present in `buf`.
pub fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Extract the `Content-Length` value from raw HTTP request headers.
pub fn parse_content_length(head: &str) -> usize {
    for line in head.split("\r\n") {
        if let Some(v) = line.strip_prefix("Content-Length:") {
            return v.trim().parse().unwrap_or(0);
        }
    }
    0
}

/// True once the accumulated buffer holds the full headers and, if a
/// `Content-Length` is declared, the full body.
pub fn request_complete(buf: &[u8]) -> bool {
    let header_end = match find_header_end(buf) {
        Some(i) => i,
        None => return false,
    };
    let head = core::str::from_utf8(&buf[..header_end]).unwrap_or("");
    let cl = parse_content_length(head);
    buf.len().saturating_sub(header_end + 4) >= cl
}

/// Parse an accumulated HTTP request buffer into method/path/body.
///
/// Returns `None` if the buffer doesn't yet contain the full headers.
pub fn parse_request(buf: &[u8]) -> Option<ParsedRequest<'_>> {
    let header_end = find_header_end(buf)?;
    let head_str = core::str::from_utf8(&buf[..header_end]).ok()?;
    let request_line = head_str.split("\r\n").next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    let content_length = parse_content_length(head_str);

    let body_start = header_end + 4;
    let body_avail = buf.len().saturating_sub(body_start);
    let body_len = content_length.min(body_avail);
    Some(ParsedRequest {
        method,
        path,
        content_length,
        body: &buf[body_start..body_start + body_len],
    })
}

/// Map a (method, path) to a [`Route`].
pub fn route(method: &str, path: &str) -> Route {
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => Route::Index,
        ("GET", "/config") => Route::GetConfig,
        ("POST", "/config") => Route::PostConfig,
        _ => Route::NotFound,
    }
}

/// Validate a `POST /config` body against `max` (max storable size).
pub fn validate_config_post(body: &[u8], max: usize) -> ConfigPostResult {
    if body.len() > max {
        return ConfigPostResult::TooLarge;
    }
    let body_str = core::str::from_utf8(body).unwrap_or("");
    match parse_config(body_str) {
        Ok(cfg) if !cfg.is_empty() => ConfigPostResult::Accepted,
        Ok(_) => ConfigPostResult::NoEffects,
        Err(_) => ConfigPostResult::BadJson,
    }
}

/// The JSON body returned for a `POST /config` outcome.
pub fn config_post_body(result: ConfigPostResult) -> &'static [u8] {
    match result {
        ConfigPostResult::Accepted => b"{\"ok\":true}",
        ConfigPostResult::TooLarge => b"{\"ok\":false,\"error\":\"too large\"}",
        ConfigPostResult::NoEffects => b"{\"ok\":false,\"error\":\"no effects\"}",
        ConfigPostResult::BadJson => b"{\"ok\":false,\"error\":\"bad json\"}",
    }
}

/// The HTTP status line for a `POST /config` outcome.
pub fn config_post_status(result: ConfigPostResult) -> &'static str {
    match result {
        ConfigPostResult::Accepted => "200 OK",
        ConfigPostResult::TooLarge => "413 Payload Too Large",
        ConfigPostResult::NoEffects | ConfigPostResult::BadJson => "400 Bad Request",
    }
}

/// Build an HTTP/1.1 response header (with `Content-Length` and
/// `Connection: close`) into `out`, returning the number of bytes written.
///
/// `out` must be at least [`RESPONSE_HEADER_BYTES`] long.
pub const RESPONSE_HEADER_BYTES: usize = 192;

pub fn build_response_header(
    out: &mut [u8],
    status: &str,
    content_type: &str,
    body_len: usize,
) -> usize {
    let mut pos = 0;
    for chunk in [
        b"HTTP/1.1 ",
        status.as_bytes(),
        b"\r\nContent-Type: ",
        content_type.as_bytes(),
        b"\r\nContent-Length: ",
    ] {
        out[pos..pos + chunk.len()].copy_from_slice(chunk);
        pos += chunk.len();
    }
    pos += write_uint(&mut out[pos..], body_len);
    let tail = b"\r\nConnection: close\r\n\r\n";
    out[pos..pos + tail.len()].copy_from_slice(tail);
    pos + tail.len()
}

/// Serialize an effect list to compact JSON (for the web UI `Save`).
/// Returns `None` if serialization would need the heap and it is unavailable,
/// but in practice always `Some` for valid configs.
#[allow(clippy::result_unit_err)]
pub fn config_to_json(cfg: &crate::config::LedConfig) -> Result<String, ()> {
    serde_json::to_string(cfg).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn req(line: &str, extra_headers: &str, body: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(line.as_bytes());
        v.extend_from_slice(b"Host: REDACTED_LAN_IP\r\n");
        v.extend_from_slice(extra_headers.as_bytes());
        v.extend_from_slice(b"\r\n");
        v.extend_from_slice(body.as_bytes());
        v
    }

    #[test]
    fn write_uint_values() {
        let mut b = [0u8; 20];
        assert_eq!(write_uint(&mut b, 0), 1);
        assert_eq!(&b[..1], b"0");
        assert_eq!(write_uint(&mut b, 5), 1);
        assert_eq!(&b[..1], b"5");
        assert_eq!(write_uint(&mut b, 42), 2);
        assert_eq!(&b[..2], b"42");
        assert_eq!(write_uint(&mut b, 14784), 5);
        assert_eq!(&b[..5], b"14784");
        assert_eq!(write_uint(&mut b, 1_000_000_000), 10);
        assert_eq!(&b[..10], b"1000000000");
    }

    #[test]
    fn find_header_end_locates_terminator() {
        let buf = b"GET / HTTP/1.1\r\nHost: x\r\n\r\n";
        // "GET / HTTP/1.1" = 14, "\r\n" = 2, "Host: x" = 7 → the "\r\n\r\n" begins
        // at index 14 + 2 + 7 = 23.
        assert_eq!(find_header_end(buf), Some(23));
        // Body starts right after the 4-byte terminator (here, empty body).
        assert_eq!(&buf[23..27], b"\r\n\r\n");
        assert_eq!(&buf[27..], b"");
    }

    #[test]
    fn find_header_end_none_when_incomplete() {
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\nHo"), None);
    }

    #[test]
    fn parse_content_length_works() {
        assert_eq!(parse_content_length("Content-Length: 5"), 5);
        assert_eq!(parse_content_length("Content-Length:  42"), 42);
        assert_eq!(parse_content_length("Content-Length: abc"), 0);
        assert_eq!(parse_content_length("Host: x"), 0);
        assert_eq!(parse_content_length("Host: x\r\nContent-Length: 7\r\n"), 7);
    }

    #[test]
    fn request_complete_get_no_body() {
        let buf = b"GET / HTTP/1.1\r\nHost: x\r\n\r\n";
        assert!(request_complete(buf));
    }

    #[test]
    fn request_complete_post_waits_for_body() {
        let full = req("POST /config HTTP/1.1", "Content-Length: 3\r\n", "abc");
        assert!(request_complete(&full));

        // Truncated body: not complete.
        let mut partial = full.clone();
        partial.truncate(partial.len() - 1); // drop last body byte
        assert!(!request_complete(&partial));

        // Missing the final \r\n\r\n: not complete.
        let mut no_term = full.clone();
        no_term.truncate(24);
        assert!(!request_complete(&no_term));
    }

    #[test]
    fn parse_request_get() {
        let buf = req("GET /config HTTP/1.1", "", "");
        let p = parse_request(&buf).unwrap();
        assert_eq!(p.method, "GET");
        assert_eq!(p.path, "/config");
        assert_eq!(p.content_length, 0);
        assert_eq!(p.body, b"");
    }

    #[test]
    fn parse_request_post_body() {
        let body = "{\"effects\":[]}";
        let buf = req("POST /config HTTP/1.1", "Content-Length: 15\r\n", body);
        let p = parse_request(&buf).unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.path, "/config");
        assert_eq!(p.content_length, 15);
        assert_eq!(p.body, body.as_bytes());
    }

    #[test]
    fn parse_request_none_when_incomplete() {
        assert!(parse_request(b"GET / HTTP/1.1\r\nHos").is_none());
    }

    #[test]
    fn routing_table() {
        assert_eq!(route("GET", "/"), Route::Index);
        assert_eq!(route("GET", "/index.html"), Route::Index);
        assert_eq!(route("GET", "/config"), Route::GetConfig);
        assert_eq!(route("POST", "/config"), Route::PostConfig);
        assert_eq!(route("GET", "/nope"), Route::NotFound);
        assert_eq!(route("POST", "/"), Route::NotFound);
        assert_eq!(route("DELETE", "/config"), Route::NotFound);
    }

    #[test]
    fn validate_config_post_accepted() {
        let body = b"{\"effects\":[{\"type\":\"blink\",\"colors\":[[1,2,3]],\"duration_ms\":10}]}";
        assert_eq!(validate_config_post(body, 4096), ConfigPostResult::Accepted);
    }

    #[test]
    fn validate_config_post_too_large() {
        let body = b"{\"effects\":[]}";
        assert_eq!(validate_config_post(body, 3), ConfigPostResult::TooLarge);
    }

    #[test]
    fn validate_config_post_no_effects() {
        assert_eq!(
            validate_config_post(b"{\"effects\":[]}", 4096),
            ConfigPostResult::NoEffects
        );
    }

    #[test]
    fn validate_config_post_bad_json() {
        assert_eq!(
            validate_config_post(b"{not json", 4096),
            ConfigPostResult::BadJson
        );
    }

    #[test]
    fn validate_config_post_non_utf8_is_bad_json() {
        assert_eq!(
            validate_config_post(&[0xff, 0xfe, 0x00], 4096),
            ConfigPostResult::BadJson
        );
    }

    #[test]
    fn post_body_and_status_consistent() {
        for r in [
            ConfigPostResult::Accepted,
            ConfigPostResult::TooLarge,
            ConfigPostResult::NoEffects,
            ConfigPostResult::BadJson,
        ] {
            // Each outcome maps to a well-formed JSON body and an HTTP status.
            let b = config_post_body(r);
            let status = config_post_status(r);
            assert!(status.starts_with(|c: char| c.is_ascii_digit()));
            let parsed: serde_json::Value =
                serde_json::from_slice(b).expect("post body must be valid JSON");
            match r {
                ConfigPostResult::Accepted => {
                    assert!(status.starts_with("200"));
                    assert_eq!(parsed["ok"], true);
                }
                _ => {
                    assert!(status.starts_with("4"));
                    assert_eq!(parsed["ok"], false);
                    assert!(parsed.get("error").is_some());
                }
            }
        }
    }

    #[test]
    fn build_response_header_has_content_length_and_close() {
        let mut h = [0u8; RESPONSE_HEADER_BYTES];
        let n = build_response_header(&mut h, "200 OK", "text/html", 14784);
        let s = core::str::from_utf8(&h[..n]).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("Content-Type: text/html\r\n"));
        assert!(s.contains("Content-Length: 14784\r\n"));
        assert!(s.contains("Connection: close\r\n"));
        assert!(s.ends_with("\r\n\r\n"));
    }

    #[test]
    fn build_response_header_zero_body() {
        let mut h = [0u8; RESPONSE_HEADER_BYTES];
        let n = build_response_header(&mut h, "404 Not Found", "text/html", 0);
        let s = core::str::from_utf8(&h[..n]).unwrap();
        assert!(s.contains("Content-Length: 0\r\n"));
    }

    #[test]
    fn config_to_json_round_trips() {
        let cfg = parse_config(
            r#"{"effects":[{"type":"blink","colors":[[1,2,3],[4,5,6]],"duration_ms":100}]}"#,
        )
        .unwrap();
        let json = config_to_json(&cfg).unwrap();
        let cfg2 = parse_config(&json).unwrap();
        // One effect with two colors survives the round trip.
        assert_eq!(cfg2.effects.len(), 1);
        match &cfg2.effects[0] {
            crate::config::LedEffect::Blink {
                colors,
                duration_ms,
            } => {
                assert_eq!(colors, &[[1, 2, 3], [4, 5, 6]]);
                assert_eq!(*duration_ms, 100);
            }
            other => panic!("expected blink, got {:?}", other),
        }
    }
}
