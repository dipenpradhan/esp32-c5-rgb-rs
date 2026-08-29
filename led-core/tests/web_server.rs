//! Integration test: overall behavior of the on-device **web server**.
//!
//! This drives the *real* `led-core::http` pipeline the firmware uses, feeding
//! it raw HTTP byte streams the same way TCP would deliver them — including
//! requests split across multiple reads (partial headers, partial bodies). It
//! simulates a full client session:
//!
//! 1. `GET /`            → the UI page (Content-Length, Connection: close).
//! 2. `GET /config`      → the current config JSON.
//! 3. `POST /config`     → a valid new config (accepted; stored).
//! 4. `GET /config`      → returns the *new* config (proving the store updated).
//! 5. `POST /config`     → invalid / too-large bodies → 400 / 413.
//!
//! The "store" here is a `Vec<u8>` mirroring the firmware's `CONFIG_DATA`.
//! What is tested is exactly the logic that runs on the device.

use led_core::config;
use led_core::http::{
    build_response_header, config_post_body, config_post_status, parse_request, request_complete,
    route, validate_config_post, Route,
};

/// The firmware's storable-config cap (matches `CONFIG_MAX` in web_server.rs).
const CONFIG_MAX: usize = 4096;

/// A built HTTP response (what the firmware would put on the wire).
#[derive(Debug)]
struct Response {
    status: String,
    content_type: String,
    body: Vec<u8>,
}

impl Response {
    /// The exact bytes on the wire: header (with Content-Length) + body.
    fn wire_bytes(&self) -> Vec<u8> {
        let mut header = [0u8; led_core::http::RESPONSE_HEADER_BYTES];
        let n = build_response_header(
            &mut header,
            &self.status,
            &self.content_type,
            self.body.len(),
        );
        let mut out = Vec::new();
        out.extend_from_slice(&header[..n]);
        out.extend_from_slice(&self.body);
        out
    }
}

/// Simulate the firmware's request-accumulation + dispatch.
///
/// `stream` is the full request bytes; they are fed one byte at a time (the
/// worst-case fragmentation) so we exercise the partial-read accumulation.
/// `store` is the current config (mirrors `CONFIG_DATA`/`CONFIG_LEN`).
/// Returns the response the server would send, and the (possibly updated) store.
fn serve(stream: &[u8], store: &mut Vec<u8>) -> Response {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    // Feed one byte at a time, accumulating until request_complete (mirrors
    // the firmware's read loop).
    for &b in stream {
        buf.push(b);
        if request_complete(&buf) {
            break;
        }
    }
    let req = parse_request(&buf).expect("accumulated buffer must parse");

    match route(req.method, req.path) {
        Route::Index => Response {
            status: "200 OK".into(),
            content_type: "text/html".into(),
            body: b"<!DOCTYPE html><html><body>LED UI</body></html>".to_vec(),
        },
        Route::GetConfig => Response {
            status: "200 OK".into(),
            content_type: "application/json".into(),
            body: store.clone(),
        },
        Route::PostConfig => {
            let result = validate_config_post(req.body, CONFIG_MAX);
            if result == led_core::http::ConfigPostResult::Accepted {
                // Firmware: store the new config bytes, bump version.
                store.clear();
                store.extend_from_slice(req.body);
            }
            Response {
                status: config_post_status(result).into(),
                content_type: "application/json".into(),
                body: config_post_body(result).to_vec(),
            }
        }
        Route::NotFound => Response {
            status: "404 Not Found".into(),
            content_type: "text/html".into(),
            body: b"HTTP/1.1 404 Not Found\r\n\r\n".to_vec(),
        },
    }
}

/// Build a raw HTTP request byte stream (headers + body).
fn http_request(method: &str, path: &str, body: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(format!("{method} {path} HTTP/1.1\r\n").as_bytes());
    v.extend_from_slice(b"Host: REDACTED_LAN_IP\r\n");
    if !body.is_empty() {
        v.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    }
    v.extend_from_slice(b"\r\n");
    v.extend_from_slice(body.as_bytes());
    v
}

#[test]
fn get_index_serves_page_with_correct_headers() {
    let mut store = Vec::new();
    let resp = serve(&http_request("GET", "/", ""), &mut store);
    assert_eq!(resp.status, "200 OK");
    assert_eq!(resp.content_type, "text/html");

    // The wire bytes must carry a correct Content-Length matching the body and
    // a Connection: close (the fix for the original client-hang bug).
    let wire = resp.wire_bytes();
    let s = core::str::from_utf8(&wire).unwrap();
    assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
    let expected_cl = format!("Content-Length: {}\r\n", resp.body.len());
    assert!(s.contains(&expected_cl));
    assert!(s.contains("Connection: close\r\n"));
    // The header terminates with \r\n\r\n and the body follows immediately.
    let boundary = s.find("\r\n\r\n").expect("header terminator present");
    assert_eq!(&s[boundary..boundary + 4], "\r\n\r\n");
    assert_eq!(
        &s[boundary + 4..],
        core::str::from_utf8(&resp.body).unwrap()
    );
}

#[test]
fn get_config_returns_current_store() {
    let current = r#"{"effects":[{"type":"blink","colors":[[1,2,3]],"duration_ms":100}]}"#;
    let mut store = current.as_bytes().to_vec();

    let resp = serve(&http_request("GET", "/config", ""), &mut store);
    assert_eq!(resp.status, "200 OK");
    assert_eq!(resp.content_type, "application/json");
    // The body is exactly the stored config.
    assert_eq!(resp.body, current.as_bytes());
}

#[test]
fn post_valid_config_then_get_returns_new_config() {
    // Start with one effect.
    let initial = r#"{"effects":[{"type":"blink","colors":[[9,9,9]],"duration_ms":1}]}"#;
    let mut store = initial.as_bytes().to_vec();

    // POST a new, valid config.
    let new_cfg =
        r#"{"effects":[{"type":"blend","from":[255,0,0],"to":[0,255,0],"steps":5,"step_ms":50}]}"#;
    let resp = serve(&http_request("POST", "/config", new_cfg), &mut store);
    assert_eq!(resp.status, "200 OK");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(body["ok"], true);

    // The store must now hold the new config (version-bumped in the firmware).
    assert_eq!(store, new_cfg.as_bytes());

    // A subsequent GET /config returns the NEW config (proves live update).
    let resp = serve(&http_request("GET", "/config", ""), &mut store);
    assert_eq!(resp.body, new_cfg.as_bytes());
}

#[test]
fn post_invalid_config_rejected_400_and_store_unchanged() {
    let initial = r#"{"effects":[{"type":"blink","colors":[[9,9,9]],"duration_ms":1}]}"#;
    let mut store = initial.as_bytes().to_vec();

    // Malformed JSON.
    let resp = serve(&http_request("POST", "/config", "{not json"), &mut store);
    assert_eq!(resp.status, "400 Bad Request");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(body["ok"], false);
    // Store untouched.
    assert_eq!(store, initial.as_bytes());

    // Valid JSON but no effects.
    let resp = serve(
        &http_request("POST", "/config", r#"{"effects":[]}"#),
        &mut store,
    );
    assert_eq!(resp.status, "400 Bad Request");
    assert_eq!(store, initial.as_bytes());
}

#[test]
fn post_too_large_rejected_413() {
    let mut store = Vec::new();
    // A config that is valid JSON + has effects, but exceeds CONFIG_MAX.
    // Pad with many blink colors so it's > 4096 bytes (~8.5 bytes/color → 600).
    let colors: Vec<String> = (0..600).map(|i| format!("[{i},0,0]")).collect();
    let big = format!(
        r#"{{"effects":[{{"type":"blink","colors":[{}],"duration_ms":1}}]}}"#,
        colors.join(",")
    );
    assert!(big.len() > CONFIG_MAX, "test fixture must exceed the cap");

    let resp = serve(&http_request("POST", "/config", &big), &mut store);
    assert_eq!(resp.status, "413 Payload Too Large");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
    assert_eq!(body["error"], "too large");
    assert!(
        store.is_empty(),
        "store must stay empty after a rejected POST"
    );
}

#[test]
fn unknown_path_is_404() {
    let mut store = Vec::new();
    let resp = serve(&http_request("GET", "/nope", ""), &mut store);
    assert_eq!(resp.status, "404 Not Found");
}

#[test]
fn full_config_lifecycle_matches_firmware_contract() {
    // End-to-end: the config a browser would POST (built from the UI's shape)
    // round-trips through the store and back out via GET, and the LED-side
    // parser accepts it.
    let mut store = Vec::new();

    // Browser POSTs (UI "Save to Device" shape).
    let ui_json = r#"{"effects":[{"type":"blink","colors":[[255,0,0],[0,0,255]],"duration_ms":400},{"type":"blend","from":[0,0,255],"to":[255,255,0],"steps":10,"step_ms":80}]}"#;
    let resp = serve(&http_request("POST", "/config", ui_json), &mut store);
    assert_eq!(resp.status, "200 OK");

    // GET it back.
    let resp = serve(&http_request("GET", "/config", ""), &mut store);
    assert_eq!(resp.body, ui_json.as_bytes());

    // The LED task would parse this stored config successfully.
    let parsed = config::parse_config(core::str::from_utf8(&store).unwrap()).unwrap();
    assert_eq!(parsed.effects.len(), 2);
}
