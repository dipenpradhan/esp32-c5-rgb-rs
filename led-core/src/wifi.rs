//! WiFi credentials config.
//!
//! Credentials live in a single config file — `configs/wifi.json` — so they
//! are not scattered as magic constants through the code. The firmware parses
//! and validates this module's model before touching the WiFi driver.
//!
//! ```json
//! { "ssid": "D", "password": "..." }
//! ```

use alloc::string::String;

/// WiFi credentials.
#[derive(Debug, serde::Deserialize)]
pub struct WifiCreds {
    /// SSID to connect to (1–32 bytes for ESP32).
    pub ssid: String,
    /// WPA2-PSK password (8–63 chars), or empty for an open network.
    pub password: String,
}

impl WifiCreds {
    /// Validate the credentials against ESP32 WiFi constraints.
    pub fn validate(&self) -> Result<(), WifiError> {
        let ssid_bytes = self.ssid.len();
        if !(1..=32).contains(&ssid_bytes) {
            return Err(WifiError::SsidLength(ssid_bytes));
        }
        let pw_len = self.password.len();
        if !self.password.is_empty() && !(8..=63).contains(&pw_len) {
            return Err(WifiError::PasswordLength(pw_len));
        }
        Ok(())
    }
}

/// A credential constraint violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiError {
    /// SSID must be 1–32 bytes.
    SsidLength(usize),
    /// WPA2-PSK password must be 8–63 chars (or empty for open networks).
    PasswordLength(usize),
}

/// The shipped default WiFi config (`configs/wifi.json`).
pub const WIFI_CONFIG_JSON: &str = include_str!("../../configs/wifi.json");

/// Parse WiFi credentials from JSON.
pub fn parse_wifi_config(json: &str) -> Result<WifiCreds, serde_json::Error> {
    serde_json::from_str(json)
}

/// Parse and validate the shipped default WiFi config.
pub fn default_wifi_config() -> Result<WifiCreds, serde_json::Error> {
    parse_wifi_config(WIFI_CONFIG_JSON)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `{"ssid":<ssid>,"password":<password>}` JSON string.
    /// Plain escaped strings (no raw strings) to avoid quote-delimiter fragility.
    fn creds_json(ssid: &str, password: &str) -> String {
        let mut s = String::from("{\"ssid\":\"");
        s.push_str(ssid);
        s.push_str("\",\"password\":\"");
        s.push_str(password);
        s.push_str("\"}");
        s
    }

    #[test]
    fn shipped_config_parses_and_is_valid() {
        let creds = default_wifi_config().expect("configs/wifi.json must be valid");
        creds
            .validate()
            .expect("shipped credentials must be within ESP32 limits");
        assert!(!creds.ssid.is_empty());
    }

    #[test]
    fn parses_ssid_and_password() {
        let creds = parse_wifi_config(&creds_json("home", "hunter222")).unwrap();
        assert_eq!(creds.ssid, "home");
        assert_eq!(creds.password, "hunter222");
        assert!(creds.validate().is_ok());
    }

    #[test]
    fn empty_password_is_open_network_ok() {
        let creds = parse_wifi_config(&creds_json("open", "")).unwrap();
        assert!(creds.validate().is_ok());
    }

    #[test]
    fn ssid_too_long_fails() {
        let creds = parse_wifi_config(&creds_json(&"x".repeat(33), "abcdef12")).unwrap();
        assert_eq!(creds.validate(), Err(WifiError::SsidLength(33)));
    }

    #[test]
    fn ssid_32_chars_passes() {
        let creds = parse_wifi_config(&creds_json(&"x".repeat(32), "abcdef12")).unwrap();
        assert!(creds.validate().is_ok());
    }

    #[test]
    fn empty_ssid_fails() {
        let creds = parse_wifi_config(&creds_json("", "abcdef12")).unwrap();
        assert_eq!(creds.validate(), Err(WifiError::SsidLength(0)));
    }

    #[test]
    fn password_too_short_fails() {
        let creds = parse_wifi_config(&creds_json("D", "short")).unwrap();
        assert_eq!(creds.validate(), Err(WifiError::PasswordLength(5)));
    }

    #[test]
    fn password_7_and_64_chars_fail() {
        let creds = parse_wifi_config(&creds_json("D", "1234567")).unwrap();
        assert_eq!(creds.validate(), Err(WifiError::PasswordLength(7)));
        let creds = parse_wifi_config(&creds_json("D", &"p".repeat(64))).unwrap();
        assert_eq!(creds.validate(), Err(WifiError::PasswordLength(64)));
    }

    #[test]
    fn password_8_and_63_chars_pass() {
        let creds = parse_wifi_config(&creds_json("D", "12345678")).unwrap();
        assert!(creds.validate().is_ok());
        let creds = parse_wifi_config(&creds_json("D", &"p".repeat(63))).unwrap();
        assert!(creds.validate().is_ok());
    }

    #[test]
    fn missing_fields_fail() {
        assert!(parse_wifi_config(r#"{ "ssid": "D" }"#).is_err());
        assert!(parse_wifi_config(r#"{ "password": "abcdef12" }"#).is_err());
    }
}
