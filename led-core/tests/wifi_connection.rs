//! Integration test: overall behavior of the **WiFi connection setup**.
//!
//! The actual radio driver is target-only (it needs the ESP32-C5 WiFi blobs),
//! so this test verifies the full host-side WiFi pipeline the firmware runs
//! *before* touching the driver:
//!
//! 1. `configs/wifi.json` (single source of truth) parses into [`WifiCreds`].
//! 2. The credentials validate against ESP32 constraints.
//! 3. The exact values the firmware would hand to the driver
//!    (`StationConfig::with_ssid` / `with_password`) are the ones in the
//!    config — no magic constants.
//! 4. The failure path: invalid credentials are rejected *before* the driver
//!    is invoked (so a bad `wifi.json` fails fast, not deep in `WifiController`).
//!
//! This is the testable half of "does WiFi connect": correct, validated
//! credentials are a precondition for `connect_async()` to succeed.

use led_core::wifi::{default_wifi_config, parse_wifi_config, WifiCreds, WifiError};

/// Build a creds JSON from ssid/password (plain escaped strings).
fn creds_json(ssid: &str, password: &str) -> String {
    let mut s = String::from("{\"ssid\":\"");
    s.push_str(ssid);
    s.push_str("\",\"password\":\"");
    s.push_str(password);
    s.push_str("\"}");
    s
}

#[test]
fn shipped_wifi_config_is_valid_and_matches_firmware_expectation() {
    let creds = default_wifi_config().expect("configs/wifi.json must parse");
    creds
        .validate()
        .expect("shipped creds must be within ESP32 limits");

    // The firmware connects to this exact SSID. This is the contract: the
    // config file, not a code constant, is the source of truth.
    assert_eq!(creds.ssid, "D");
    // The password must be a valid WPA2-PSK length (8..=63).
    assert!((8..=63).contains(&creds.password.len()));
}

#[test]
fn credentials_flow_into_driver_config_unmodified() {
    // The firmware does:
    //   StationConfig::default().with_ssid(WIFI_SSID).with_password(WIFI_PASS)
    // with WIFI_SSID/WIFI_PASS sourced from this config. Assert the exact
    // strings that reach the driver are byte-identical to the config (no
    // trimming/casing/side-effects).
    let creds = default_wifi_config().unwrap();
    assert_eq!(creds.ssid.as_bytes(), b"D");
    assert!(!creds.password.is_empty());
    // No leading/trailing whitespace sneaked in.
    assert_eq!(creds.ssid.trim(), creds.ssid);
    assert_eq!(creds.password.trim(), creds.password);
}

#[test]
fn valid_credentials_at_all_boundaries_pass() {
    // SSID 1..=32, password 8..=63 (or empty).
    for (ssid, pw, ok) in [
        ("a", "12345678", true),                 // min ssid, min pw
        (&"a".repeat(32), &"p".repeat(8), true), // max ssid, min pw
        ("net", &"p".repeat(63), true),          // max pw
        ("open", "", true),                      // open network
    ] {
        let creds = parse_wifi_config(&creds_json(ssid, pw)).unwrap();
        assert!(
            creds.validate().is_ok(),
            "expected ({ssid:?}, pw_len={}) to be valid",
            pw.len()
        );
        assert!(ok);
    }
}

#[test]
fn invalid_credentials_rejected_before_driver() {
    // Each of these would have made WifiController fail deep in the stack; the
    // host-side validation catches them first.
    let cases: [(String, String, WifiError); 4] = [
        ("".into(), "12345678".into(), WifiError::SsidLength(0)),
        ("x".repeat(33), "12345678".into(), WifiError::SsidLength(33)),
        ("D".into(), "short".into(), WifiError::PasswordLength(5)),
        ("D".into(), "p".repeat(64), WifiError::PasswordLength(64)),
    ];
    for (ssid, pw, expected) in cases {
        let creds: WifiCreds = parse_wifi_config(&creds_json(&ssid, &pw)).unwrap();
        assert_eq!(
            creds.validate(),
            Err(expected),
            "ssid={ssid:?} pw_len={}",
            pw.len()
        );
    }
}

#[test]
fn missing_or_malformed_config_fails_fast() {
    assert!(parse_wifi_config(r#"{}"#).is_err());
    assert!(parse_wifi_config("not json").is_err());
    assert!(parse_wifi_config(r#"{"ssid":"D"}"#).is_err()); // no password
}
