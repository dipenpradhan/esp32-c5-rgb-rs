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
    /// The body is valid JSON with ≥1 effect, fits in `max`, and every field
    /// is within the sanity bounds in [`validate_config_post`].
    Accepted,
    /// The body is too large to store.
    TooLarge,
    /// The body is valid JSON but has no effects.
    NoEffects,
    /// The body is not valid config JSON.
    BadJson,
    /// The body is valid JSON with ≥1 effect, but some field falls outside the
    /// sanity bounds (e.g. `steps` too large, `steps == 0`, too many effects
    /// or colors, or a duration too large). Distinct from [`BadJson`] so the
    /// web UI can show the user that their JSON parsed but a value was
    /// rejected, rather than a misleading "bad json".
    OutOfRange,
}

/// Upper bound on the decimal width of a `usize` — the largest buffer
/// [`write_uint`] can ever need.
pub const UINT_BUF_BYTES: usize = 20; // enough for a 64-bit usize

/// Write `n` as decimal ASCII into `buf`; returns the number of bytes written.
///
/// `buf` must hold `n`'s decimal digits: [`UINT_BUF_BYTES`] is always enough,
/// but only as many bytes as `n` actually has are touched, so a caller that
/// knows `n`'s width may pass a shorter slice (`build_response_header` does).
/// Panics if `buf` is shorter than that.
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

/// Number of decimal ASCII digits `n` needs (i.e. what `write_uint` writes).
fn digit_count(n: usize) -> usize {
    let mut n = n;
    let mut digits = 1;
    while n >= 10 {
        n /= 10;
        digits += 1;
    }
    digits
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

/// Sanity bounds for a `POST /config` body. These are network-reachable
/// limits: an attacker (or a buggy UI) can otherwise make the LED task's
/// `cycle_steps` expand a config into a `Vec<Step>` with billions of elements
/// on a device whose heap is 8 KiB–100 KiB — a trivial out-of-memory denial
/// of service. Each bound is chosen so a legitimate human config always fits
/// while the worst-case expansion stays well under the *smallest* heap:
///
/// - [`MAX_TOTAL_FRAMES`] is the real heap guard: `cycle_steps` builds one
///   `Vec<Step>` for the whole cycle, and a `Step` is 8 bytes, so 512 frames
///   = 4096 bytes (4 KiB) — fits the 8 KiB minimum heap with headroom.
/// - The per-effect/per-field bounds keep any single field from being absurd
///   on its own (they are the *strongest* constraints when only one effect
///   is misconfigured, but the total-frame bound is what caps the allocation).
const MAX_EFFECTS: usize = 32;
/// Most effects a human would sequence; well below the total-frame cap.
const MAX_COLORS_PER_EFFECT: usize = 64;
/// A blend's `steps`: `steps + 1` frames; 64 keeps a single blend to ≤65 frames.
const MAX_STEPS_PER_BLEND: u32 = 64;
/// Total frames one full cycle may expand to. `512 * size_of::<Step>()`
/// (8 bytes) = 4096 bytes (4 KiB), which fits the 8 KiB minimum device heap
/// with headroom; a hand-tuned config of a few effects sits far below this.
const MAX_TOTAL_FRAMES: usize = 512;
/// Maximum hold per frame (ms). A human configures second-scale holds; 1
/// minute is a generous ceiling. Beyond it the LED looks frozen, and a u32
/// max (~49 days) is clearly not a sane per-frame hold.
const MAX_DURATION_MS: u32 = 60_000;

/// True if every field of `cfg` is within the sanity bounds in
/// [`MAX_EFFECTS`]/[`MAX_COLORS_PER_EFFECT`]/[`MAX_STEPS_PER_BLEND`]/
/// [`MAX_TOTAL_FRAMES`]/[`MAX_DURATION_MS`]. Called only after the config has
/// parsed and has ≥1 effect.
///
/// The cheap per-field bounds are checked *before* the total-frame sum: that
/// way `LedEffect::frame_count()` (which does `steps + 1`) is only ever called
/// once every `steps` is known to be ≤ [`MAX_STEPS_PER_BLEND`], so the sum is
/// bounded by `MAX_EFFECTS × (MAX_STEPS_PER_BLEND + 1)` and cannot overflow
/// `usize` even on a 32-bit target. Summing first would let a malicious
/// `steps: u32::MAX` overflow `frame_count` on the device during validation.
fn config_in_bounds(cfg: &crate::config::LedConfig) -> bool {
    if cfg.effects.len() > MAX_EFFECTS {
        return false;
    }
    for e in &cfg.effects {
        match e {
            crate::config::LedEffect::Blink {
                colors,
                duration_ms,
            } => {
                if colors.len() > MAX_COLORS_PER_EFFECT || *duration_ms > MAX_DURATION_MS {
                    return false;
                }
            }
            crate::config::LedEffect::Blend { steps, step_ms, .. } => {
                // `steps == 0` is accepted by the parser but makes the web UI
                // compute 0/0 = NaN and freeze its preview; reject it here.
                if *steps < 1 || *steps > MAX_STEPS_PER_BLEND || *step_ms > MAX_DURATION_MS {
                    return false;
                }
            }
        }
    }
    // The real heap guard: bound the total frames `cycle_steps` will allocate
    // into its `Vec<Step>`. Safe to sum here because the per-field bounds above
    // already cap each effect's frame count.
    let total_frames: usize = cfg.effects.iter().map(|e| e.frame_count()).sum();
    total_frames <= MAX_TOTAL_FRAMES
}

/// Validate a `POST /config` body against `max` (max storable size).
///
/// Checks, in order: body size, UTF-8 + JSON parseability, non-empty, and the
/// field-range sanity bounds in [`config_in_bounds`]. The size/parse checks
/// return the pre-existing [`ConfigPostResult`] variants; a body that parses
/// and has ≥1 effect but fails the range bounds returns [`OutOfRange`].
pub fn validate_config_post(body: &[u8], max: usize) -> ConfigPostResult {
    if body.len() > max {
        return ConfigPostResult::TooLarge;
    }
    let body_str = core::str::from_utf8(body).unwrap_or("");
    match parse_config(body_str) {
        Ok(cfg) if cfg.is_empty() => ConfigPostResult::NoEffects,
        Ok(cfg) if config_in_bounds(&cfg) => ConfigPostResult::Accepted,
        Ok(_) => ConfigPostResult::OutOfRange,
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
        ConfigPostResult::OutOfRange => b"{\"ok\":false,\"error\":\"value out of range\"}",
    }
}

/// The HTTP status line for a `POST /config` outcome.
pub fn config_post_status(result: ConfigPostResult) -> &'static str {
    match result {
        ConfigPostResult::Accepted => "200 OK",
        ConfigPostResult::TooLarge => "413 Payload Too Large",
        ConfigPostResult::NoEffects | ConfigPostResult::BadJson | ConfigPostResult::OutOfRange => {
            "400 Bad Request"
        }
    }
}

/// Build an HTTP/1.1 response header (with `Content-Length` and
/// `Connection: close`) into `out`, returning the number of bytes written.
///
/// Callers should pass at least [`RESPONSE_HEADER_BYTES`] (enough for any
/// status line, `Content-Type`, and a full-width `Content-Length`). With a
/// smaller `out`, the header is truncated at the buffer boundary (the
/// decimal length is written only if it fits entirely) and the returned
/// length is correspondingly smaller; the function never writes past the
/// end of `out`.
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
        // Bound every write by `out`'s length: a buffer smaller than
        // [`RESPONSE_HEADER_BYTES`] is truncated, never overflowed.
        let end = (pos + chunk.len()).min(out.len());
        out[pos..end].copy_from_slice(&chunk[..end - pos]);
        pos = end;
        if pos == out.len() {
            break;
        }
    }
    // The decimal `body_len` is written only if it fits whole: a partially
    // written number would silently change its value.
    if out.len() - pos >= digit_count(body_len) {
        pos += write_uint(&mut out[pos..], body_len);
    }
    let tail = b"\r\nConnection: close\r\n\r\n";
    let end = (pos + tail.len()).min(out.len());
    out[pos..end].copy_from_slice(&tail[..end - pos]);
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;

    fn req(line: &str, extra_headers: &str, body: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(line.as_bytes());
        v.extend_from_slice(b"Host: 127.0.0.1\r\n");
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

    /// A blend whose `steps` is large enough to overflow the old `i32` lerp
    /// and to blow the 8 KiB heap — the original DoS vector. Must be rejected
    /// as out-of-range, not accepted.
    #[test]
    fn validate_config_post_rejects_huge_blend_steps() {
        // 4.3e9 steps × 1 µs is ~49 days per frame and would make
        // `cycle_steps` try to allocate 4.3e9 × 8 bytes.
        let body = b"{\"effects\":[{\"type\":\"blend\",\"from\":[0,0,0],\"to\":[255,255,255],\"steps\":4294967295,\"step_ms\":1}]}";
        assert_eq!(
            validate_config_post(body, 4096),
            ConfigPostResult::OutOfRange
        );
    }

    /// `steps == 0` parses fine but freezes the web UI preview (0/0 = NaN).
    #[test]
    fn validate_config_post_rejects_zero_blend_steps() {
        let body = b"{\"effects\":[{\"type\":\"blend\",\"from\":[0,0,0],\"to\":[1,1,1],\"steps\":0,\"step_ms\":5}]}";
        assert_eq!(
            validate_config_post(body, 4096),
            ConfigPostResult::OutOfRange
        );
    }

    /// `duration_ms` above the human-plausible ceiling (1 min) is rejected.
    #[test]
    fn validate_config_post_rejects_huge_duration() {
        let body =
            b"{\"effects\":[{\"type\":\"blink\",\"colors\":[[1,2,3]],\"duration_ms\":4294967295}]}";
        assert_eq!(
            validate_config_post(body, 4096),
            ConfigPostResult::OutOfRange
        );
    }

    /// A blend with too many effects is rejected (effect-count bound).
    #[test]
    fn validate_config_post_rejects_too_many_effects() {
        // 33 blink effects × 1 color each = 33 < MAX_TOTAL_FRAMES, so only the
        // MAX_EFFECTS=32 bound trips.
        let effects: Vec<String> = (0..33)
            .map(|i| format!("{{\"type\":\"blink\",\"colors\":[[{i},0,0]],\"duration_ms\":1}}"))
            .collect();
        let body = format!("{{\"effects\":[{}]}}", effects.join(","));
        assert!(body.len() < 4096, "fixture must fit the size cap");
        assert_eq!(
            validate_config_post(body.as_bytes(), 4096),
            ConfigPostResult::OutOfRange
        );
    }

    /// A blink with more colors than the per-effect cap is rejected.
    #[test]
    fn validate_config_post_rejects_too_many_colors() {
        // 65 colors > MAX_COLORS_PER_EFFECT (64), but total frames (65) is
        // still < MAX_TOTAL_FRAMES (512), so this trips the per-effect bound.
        let colors: Vec<String> = (0..65).map(|i| format!("[{i},0,0]")).collect();
        let body = format!(
            "{{\"effects\":[{{\"type\":\"blink\",\"colors\":[{}],\"duration_ms\":1}}]}}",
            colors.join(",")
        );
        assert!(body.len() < 4096, "fixture must fit the size cap");
        assert_eq!(
            validate_config_post(body.as_bytes(), 4096),
            ConfigPostResult::OutOfRange
        );
    }

    /// The total-frame heap guard: a config that passes every per-field bound
    /// but still expands past `MAX_TOTAL_FRAMES` is rejected. Blends are used
    /// because they expand many frames from a few bytes of JSON (unlike blinks,
    /// whose per-color JSON size makes the 4096-byte cap hit first).
    #[test]
    fn validate_config_post_rejects_total_frame_blowout() {
        // 8 blends × 64 steps each: steps == MAX_STEPS_PER_BLEND (allowed),
        // 8 effects < MAX_EFFECTS (32, allowed), but 8 × 65 = 520 frames
        // > MAX_TOTAL_FRAMES (512) → only the total-frame bound trips.
        // Each blend is ~80 bytes of JSON, so the body is ~640 bytes (well
        // under the 4096 cap) yet would allocate 520 × 8 = 4160 bytes of
        // `Vec<Step>` in `cycle_steps`.
        let effects: Vec<String> = (0..8)
            .map(|e| {
                format!(
                    "{{\"type\":\"blend\",\"from\":[{e},0,0],\"to\":[{e},255,255],\"steps\":64,\"step_ms\":1}}"
                )
            })
            .collect();
        let body = format!("{{\"effects\":[{}]}}", effects.join(","));
        assert!(body.len() < 4096, "fixture must fit the size cap");
        assert_eq!(
            validate_config_post(body.as_bytes(), 4096),
            ConfigPostResult::OutOfRange
        );
    }

    /// A config that is right at every bound is still accepted (no off-by-one
    /// rejection of the legitimate maximum).
    #[test]
    fn validate_config_post_accepts_at_bounds() {
        // 64 colors (== MAX_COLORS_PER_EFFECT), duration 60000 (== cap),
        // 65 frames total (< 512) → Accepted.
        let colors: Vec<String> = (0..64).map(|i| format!("[{i},0,0]")).collect();
        let body = format!(
            "{{\"effects\":[{{\"type\":\"blink\",\"colors\":[{}],\"duration_ms\":60000}}]}}",
            colors.join(",")
        );
        assert_eq!(
            validate_config_post(body.as_bytes(), 4096),
            ConfigPostResult::Accepted
        );

        // Blend right at steps = 64 (== MAX_STEPS_PER_BLEND) and step_ms = 60000
        // (== cap) → Accepted (65 frames < 512).
        let blend = b"{\"effects\":[{\"type\":\"blend\",\"from\":[0,0,0],\"to\":[255,255,255],\"steps\":64,\"step_ms\":60000}]}";
        assert_eq!(
            validate_config_post(blend, 4096),
            ConfigPostResult::Accepted
        );
    }

    #[test]
    fn post_body_and_status_consistent() {
        for r in [
            ConfigPostResult::Accepted,
            ConfigPostResult::TooLarge,
            ConfigPostResult::NoEffects,
            ConfigPostResult::BadJson,
            ConfigPostResult::OutOfRange,
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
    fn build_response_header_exactly_fills_buffer() {
        // "HTTP/1.1 " (9) + "200 OK" (6) + "\r\nContent-Type: " (16) +
        // "text/html" (9) + "\r\nContent-Length: " (18) + "5" (1) +
        // "\r\nConnection: close\r\n\r\n" (23) = 82 bytes, so this header
        // fills an 82-byte buffer exactly.
        let mut h = [0u8; 82];
        let n = build_response_header(&mut h, "200 OK", "text/html", 5);
        assert_eq!(n, 82);
        let s = core::str::from_utf8(&h[..n]).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("Content-Length: 5\r\n"));
        assert!(s.ends_with("Connection: close\r\n\r\n"));
    }

    #[test]
    fn build_response_header_truncates_on_overflow() {
        // The same 82-byte header with one less byte of room: the guard
        // truncates at the buffer boundary instead of writing past it.
        let mut h = [0u8; 81];
        let n = build_response_header(&mut h, "200 OK", "text/html", 5);
        assert_eq!(n, 81);
        let s = core::str::from_utf8(&h[..n]).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        // The last "\r\n" of the `Connection: close` line is cut off.
        assert!(!s.ends_with("\r\n\r\n"));
        // The extreme case: an empty buffer writes nothing.
        assert_eq!(build_response_header(&mut [], "200 OK", "text/html", 5), 0);
    }

    #[test]
    fn build_response_header_omits_length_that_does_not_fit() {
        // The 58-byte fixed prefix ("HTTP/1.1 200 OK\r\nContent-Type:
        // text/html\r\nContent-Length: ") fits in 67 bytes, but the
        // full-width `Content-Length` value does not, so it is omitted
        // rather than written half.
        let mut h = [0u8; 67];
        let n = build_response_header(&mut h, "200 OK", "text/html", usize::MAX);
        assert_eq!(n, 67);
        let s = core::str::from_utf8(&h[..n]).unwrap();
        assert!(s.contains("Content-Length: "));
        assert!(s.ends_with("Content-Length: \r\nConnect"));
    }

    #[test]
    fn config_json_round_trips() {
        // `LedConfig` is `#[derive(Serialize, Deserialize)]`; the web UI's
        // `Save` path serializes a config and the device re-parses it, so the
        // round trip through the wire's compact JSON form must preserve the
        // effect list. This exercises the same serde derive the firmware
        // relies on (the standalone `config_to_json` helper was removed — it
        // had zero callers — so we serialize directly via serde_json).
        let cfg = parse_config(
            r#"{"effects":[{"type":"blink","colors":[[1,2,3],[4,5,6]],"duration_ms":100}]}"#,
        )
        .unwrap();
        let json = serde_json::to_string(&cfg).unwrap();
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
