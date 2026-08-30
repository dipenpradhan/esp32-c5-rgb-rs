//! Effects config model and JSON parsing.
//!
//! Mirrors the schema used by `configs/effects.json` and the web UI:
//!
//! ```json
//! {
//!   "effects": [
//!     { "type": "blink", "colors": [[255,0,0],[0,255,0]], "duration_ms": 300 },
//!     { "type": "blend", "from": [255,0,0], "to": [0,255,0], "steps": 20, "step_ms": 100 }
//!   ]
//! }
//! ```

use alloc::vec::Vec;

/// Top-level config: a list of effects to run in sequence (looping).
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct LedConfig {
    pub effects: Vec<LedEffect>,
}

impl LedConfig {
    /// An empty config (used as the fallback when parsing fails).
    pub fn empty() -> Self {
        Self {
            effects: Vec::new(),
        }
    }

    /// True if this config has at least one effect.
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }
}

/// A single effect.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type")]
pub enum LedEffect {
    /// Cycle through `colors` in order, holding each for `duration_ms`.
    #[serde(rename = "blink")]
    Blink {
        colors: Vec<[u8; 3]>,
        duration_ms: u32,
    },
    /// Linearly interpolate `from` → `to` over `steps` steps of `step_ms` each.
    #[serde(rename = "blend")]
    Blend {
        from: [u8; 3],
        to: [u8; 3],
        steps: u32,
        step_ms: u32,
    },
}

impl LedEffect {
    /// How many distinct color frames this effect emits.
    pub fn frame_count(&self) -> usize {
        match self {
            LedEffect::Blink { colors, .. } => colors.len(),
            // The blend loop runs `0..=steps`, i.e. steps + 1 frames.
            LedEffect::Blend { steps, .. } => (*steps as usize) + 1,
        }
    }

    /// Total time (ms) one pass of this effect takes.
    pub fn total_duration_ms(&self) -> u64 {
        match self {
            LedEffect::Blink {
                colors,
                duration_ms,
            } => colors.len() as u64 * *duration_ms as u64,
            LedEffect::Blend { steps, step_ms, .. } => (*steps as u64 + 1) * *step_ms as u64,
        }
    }
}

/// Parse a config from a JSON string.
pub fn parse_config(json: &str) -> Result<LedConfig, serde_json::Error> {
    serde_json::from_str(json)
}

/// Parse a config from a JSON byte slice.
pub fn parse_config_bytes(bytes: &[u8]) -> Result<LedConfig, serde_json::Error> {
    serde_json::from_slice(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped default config (configs/effects.json), embedded at build time.
    const DEFAULT_JSON: &str = include_str!("../../configs/effects.json");

    // The single "golden" test for the shipped fixture: it asserts only that
    // `configs/effects.json` *parses* and is a *valid* (non-empty) config. It
    // deliberately does NOT assert the specific effects, colors, frame counts,
    // or durations — a legitimate edit to the shipped file must not break this
    // test (that churn is exactly why the logic assertions live on synthetic
    // fixtures in the test files instead).
    #[test]
    fn shipped_config_parses_and_is_valid() {
        let cfg = parse_config(DEFAULT_JSON).expect("shipped effects.json must be valid JSON");
        assert!(
            !cfg.is_empty(),
            "shipped effects.json must contain at least one effect"
        );
    }

    #[test]
    fn blink_minimal() {
        let cfg =
            parse_config(r#"{"effects":[{"type":"blink","colors":[[1,2,3]],"duration_ms":50}]}"#)
                .unwrap();
        assert_eq!(cfg.effects.len(), 1);
        assert_eq!(cfg.effects[0].frame_count(), 1);
        assert_eq!(cfg.effects[0].total_duration_ms(), 50);
    }

    #[test]
    fn blend_frames_and_duration() {
        let cfg = parse_config(
            r#"{"effects":[{"type":"blend","from":[0,0,0],"to":[255,255,255],"steps":9,"step_ms":10}]}"#,
        )
        .unwrap();
        // 0..=9 → 10 frames, 10 × 10 ms.
        assert_eq!(cfg.effects[0].frame_count(), 10);
        assert_eq!(cfg.effects[0].total_duration_ms(), 100);
    }

    #[test]
    fn zero_steps_blend_still_emits_one_frame() {
        let cfg = parse_config(
            r#"{"effects":[{"type":"blend","from":[0,0,0],"to":[9,9,9],"steps":0,"step_ms":5}]}"#,
        )
        .unwrap();
        assert_eq!(cfg.effects[0].frame_count(), 1);
        assert_eq!(cfg.effects[0].total_duration_ms(), 5);
    }

    #[test]
    fn empty_effects_list_is_valid() {
        let cfg = parse_config(r#"{"effects":[]}"#).unwrap();
        assert!(cfg.is_empty());
        assert_eq!(cfg.effects.len(), 0);
    }

    #[test]
    fn missing_effects_field_fails() {
        assert!(parse_config(r#"{}"#).is_err());
    }

    #[test]
    fn wrong_type_tag_fails() {
        assert!(parse_config(r#"{"effects":[{"type":"rainbow"}]}"#).is_err());
    }

    #[test]
    fn unknown_type_fails() {
        assert!(parse_config(r#"{"effects":[123]}"#).is_err());
    }

    #[test]
    fn malformed_json_fails() {
        assert!(parse_config(r#"{"effects": ["#).is_err());
        assert!(parse_config("").is_err());
    }

    #[test]
    fn parse_bytes_matches_parse_str() {
        let json = r#"{"effects":[{"type":"blink","colors":[[8,9,10]],"duration_ms":1}]}"#;
        let a = parse_config(json).unwrap();
        let b = parse_config_bytes(json.as_bytes()).unwrap();
        assert_eq!(a.effects.len(), b.effects.len());
        assert_eq!(a.effects[0].frame_count(), b.effects[0].frame_count());
    }
}
