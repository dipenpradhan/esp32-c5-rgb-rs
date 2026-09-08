//! Expand a [`LedConfig`] into the flat sequence of LED steps one full cycle
//! will emit.
//!
//! This is the "overall behavior" of the LED, made host-testable: given a
//! config, [`cycle_steps`] returns exactly the (color, hold) frames the LED
//! driver will emit for one pass through all effects — in order, with the
//! exact durations. The firmware never calls this function — its loops
//! re-derive the same sequence inline from `config.effects`
//! (`src/main.rs`, `examples/web_server.rs`) — and the two are intended to
//! agree. Tests assert on the model.

use alloc::vec::Vec;

use crate::color::{interpolate, Rgb};
use crate::config::{LedConfig, LedEffect};

/// One LED step: a color to display and how long to hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// The color as `[R, G, B]` (the config/UI order).
    pub color: Rgb,
    /// How long to hold the color, in ms.
    pub hold_ms: u32,
}

/// Expand a config into the flat list of steps for one full cycle
/// (all effects, in order).
///
/// - `blink`  → one step per color, each held `duration_ms`.
/// - `blend`  → `steps + 1` steps (`0..=steps`), each interpolating
///   `from`→`to` and held `step_ms`. The endpoints land exactly on `from`
///   and `to`.
pub fn cycle_steps(config: &LedConfig) -> Vec<Step> {
    let mut out = Vec::new();
    for effect in &config.effects {
        match effect {
            LedEffect::Blink {
                colors,
                duration_ms,
            } => {
                for color in colors {
                    out.push(Step {
                        color: *color,
                        hold_ms: *duration_ms,
                    });
                }
            }
            LedEffect::Blend {
                from,
                to,
                steps,
                step_ms,
            } => {
                for step in 0..=*steps {
                    out.push(Step {
                        color: interpolate(from, to, step, *steps),
                        hold_ms: *step_ms,
                    });
                }
            }
        }
    }
    out
}

/// Total hold time (ms) of one full cycle.
pub fn cycle_duration_ms(config: &LedConfig) -> u64 {
    config.effects.iter().map(|e| e.total_duration_ms()).sum()
}

/// How many frames one full cycle emits.
pub fn cycle_frame_count(config: &LedConfig) -> usize {
    config.effects.iter().map(|e| e.frame_count()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_config;

    #[test]
    fn blink_expands_to_one_step_per_color() {
        let cfg = parse_config(
            r#"{"effects":[{"type":"blink","colors":[[255,0,0],[0,255,0],[0,0,255]],"duration_ms":300}]}"#,
        )
        .unwrap();
        let steps = cycle_steps(&cfg);
        assert_eq!(steps.len(), 3);
        assert_eq!(
            steps[0],
            Step {
                color: [255, 0, 0],
                hold_ms: 300
            }
        );
        assert_eq!(
            steps[1],
            Step {
                color: [0, 255, 0],
                hold_ms: 300
            }
        );
        assert_eq!(
            steps[2],
            Step {
                color: [0, 0, 255],
                hold_ms: 300
            }
        );
        assert_eq!(cycle_duration_ms(&cfg), 900);
        assert_eq!(cycle_frame_count(&cfg), 3);
    }

    #[test]
    fn blend_expands_to_steps_plus_one_with_exact_endpoints() {
        let cfg = parse_config(
            r#"{"effects":[{"type":"blend","from":[255,0,0],"to":[0,255,0],"steps":3,"step_ms":100}]}"#,
        )
        .unwrap();
        let steps = cycle_steps(&cfg);
        assert_eq!(steps.len(), 4, "steps+1 frames");
        // Endpoints are exact.
        assert_eq!(steps[0].color, [255, 0, 0]);
        assert_eq!(steps[3].color, [0, 255, 0]);
        // Each held step_ms.
        assert!(steps.iter().all(|s| s.hold_ms == 100));
        assert_eq!(cycle_duration_ms(&cfg), 400);
    }

    #[test]
    fn zero_step_blend_is_one_frame() {
        let cfg = parse_config(
            r#"{"effects":[{"type":"blend","from":[1,2,3],"to":[4,5,6],"steps":0,"step_ms":50}]}"#,
        )
        .unwrap();
        let steps = cycle_steps(&cfg);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].color, [1, 2, 3], "zero steps → the 'from' color");
    }

    #[test]
    fn mixed_config_preserves_order_and_accumulates() {
        let cfg = parse_config(
            r#"{"effects":[
                {"type":"blink","colors":[[255,255,255]],"duration_ms":200},
                {"type":"blend","from":[255,255,255],"to":[0,0,0],"steps":1,"step_ms":100}
            ]}"#,
        )
        .unwrap();
        let steps = cycle_steps(&cfg);
        // 1 blink step + (1+1) blend steps = 3.
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].color, [255, 255, 255]);
        assert_eq!(steps[2].color, [0, 0, 0]);
        assert_eq!(cycle_duration_ms(&cfg), 200 + 200);
    }

    #[test]
    fn empty_config_yields_no_steps() {
        let cfg = parse_config(r#"{"effects":[]}"#).unwrap();
        assert!(cycle_steps(&cfg).is_empty());
        assert_eq!(cycle_duration_ms(&cfg), 0);
        assert_eq!(cycle_frame_count(&cfg), 0);
    }

    // Synthetic fixture (not the shipped effects.json): a blink followed by a
    // blend. The logic assertions (frame expansion, frame count, duration
    // accumulation, non-zero holds) live here on a config defined in the test
    // file, so a legitimate edit to the shipped fixture cannot break them.
    #[test]
    fn synthetic_config_expands_sensibly() {
        let cfg = parse_config(
            r#"{
                "effects": [
                    {"type":"blink","colors":[[255,0,0],[0,255,0],[0,0,255]],"duration_ms":300},
                    {"type":"blend","from":[255,0,0],"to":[0,255,255],"steps":20,"step_ms":100}
                ]
            }"#,
        )
        .unwrap();
        let steps = cycle_steps(&cfg);
        // blink(3) + blend(20+1) = 24 frames.
        assert_eq!(steps.len(), 24);
        assert_eq!(cycle_frame_count(&cfg), 24);
        // 3*300 + 21*100 = 900 + 2100 = 3000 ms.
        assert_eq!(cycle_duration_ms(&cfg), 3000);
        // Every step's color and hold is well-formed.
        for s in &steps {
            assert!(s.hold_ms > 0);
        }
    }
}
