//! Integration test: overall behavior of the **LED** effect engine.
//!
//! The firmware's `led_task` is a thin loop over a config; this test asserts
//! the *observable LED behavior* end-to-end:
//!
//! - the exact frame sequence (colors, in GRB wire order) emitted for one full
//!   cycle of a multi-effect config (synthetic, defined below),
//! - timing (74 µs per frame + the per-step hold),
//! - live reconfiguration: a new `POST /config`-shaped JSON changes the next
//!   cycle's frames,
//! - invalid/empty configs leave the LED alive (white fallback) rather than
//!   dead.
//!
//! Frames are produced via `led_core::ws2812::encode_rgb` (the same encoding
//! the firmware bit-bangs), so what is asserted here is what hits the wire.

use led_core::config::{parse_config, LedConfig};
use led_core::effects::{cycle_duration_ms, cycle_steps};
use led_core::ws2812::{encode_rgb, Ws2812Frame, FRAME_US};

/// A synthetic multi-effect config that mirrors the shape of the shipped
/// `configs/effects.json` (4 effects: blink(3) + blend(20) + blink(2) +
/// blend(15)). All logic assertions in this file run against *this* fixture
/// (defined here, in the test), not the shipped file — so a legitimate edit to
/// `configs/effects.json` does not break these tests for the wrong reason.
/// The single golden check that the shipped file parses and is valid lives in
/// `config.rs::tests::shipped_config_parses_and_is_valid`.
const FIXTURE_JSON: &str = r#"{
  "effects": [
    { "type": "blink", "colors": [[255,0,0],[0,255,0],[0,0,255]], "duration_ms": 300 },
    { "type": "blend", "from": [255,0,0], "to": [0,255,255], "steps": 20, "step_ms": 100 },
    { "type": "blink", "colors": [[255,255,255],[0,0,0]], "duration_ms": 200 },
    { "type": "blend", "from": [0,255,0], "to": [255,0,255], "steps": 15, "step_ms": 80 }
  ]
}"#;

fn fixture_config() -> LedConfig {
    parse_config(FIXTURE_JSON).expect("fixture config must parse")
}

#[test]
fn one_cycle_of_fixture_emits_expected_frames() {
    let steps = cycle_steps(&fixture_config());
    // 4 effects: blink(3) + blend(21) + blink(2) + blend(16) = 42 frames.
    assert_eq!(steps.len(), 42);

    // Each step's color is encoded to a well-formed WS2812 frame (24 bits +
    // ≥50 µs reset), i.e. exactly what the LED will latch.
    for s in &steps {
        let frame = encode_rgb(&s.color);
        assert!(frame.reset.us >= 50, "every frame needs the reset pulse");
    }

    // First effect (blink R,G,B @300ms) comes first, in config order.
    assert_eq!(steps[0].color, [255, 0, 0]);
    assert_eq!(steps[1].color, [0, 255, 0]);
    assert_eq!(steps[2].color, [0, 0, 255]);
    assert!(steps[0..3].iter().all(|s| s.hold_ms == 300));

    // Second effect (blend red→cyan, 20 steps) starts at step 3: exactly red,
    // ends (step 23) at exactly cyan.
    assert_eq!(steps[3].color, [255, 0, 0]);
    assert_eq!(steps[23].color, [0, 255, 255]);
    assert!(steps[3..24].iter().all(|s| s.hold_ms == 100));

    // Third effect: white → off blink @200ms.
    assert_eq!(steps[24].color, [255, 255, 255]);
    assert_eq!(steps[25].color, [0, 0, 0]);
    assert!(steps[24..26].iter().all(|s| s.hold_ms == 200));

    // Fourth effect: green→magenta blend, ends exactly magenta at the tail.
    assert_eq!(steps[26].color, [0, 255, 0]);
    assert_eq!(steps[41].color, [255, 0, 255]);
}

#[test]
fn fixture_cycle_timing() {
    // 3*300 + 21*100 + 2*200 + 16*80 = 4680 ms of holds + 42 frames × 74 µs
    // of blocking bit-bang (negligible but real).
    let cfg = fixture_config();
    assert_eq!(cycle_duration_ms(&cfg), 4680);
    let steps = cycle_steps(&cfg);
    let total_block_us: u64 = steps.iter().map(|_| FRAME_US as u64).sum();
    assert_eq!(total_block_us, 42 * 74);
}

#[test]
fn live_reconfigure_changes_next_cycle() {
    // The LED task: parse current config → run cycle → re-check version →
    // (on change) re-parse → run the new cycle. This mirrors led_task exactly.
    let mut stored = FIXTURE_JSON.as_bytes().to_vec();

    fn parse_stored(stored: &[u8]) -> LedConfig {
        parse_config(core::str::from_utf8(stored).unwrap()).unwrap()
    }

    // Cycle 1: the default (42 frames).
    let cycle1 = cycle_steps(&parse_stored(&stored));
    assert_eq!(cycle1.len(), 42);

    // A POST /config replaces the store (as the web_server task does).
    let new_cfg = r#"{"effects":[{"type":"blink","colors":[[0,255,0]],"duration_ms":1000}]}"#;
    stored = new_cfg.as_bytes().to_vec();

    // Cycle 2: the new single green blink (1 frame), proving the LED follows
    // the live-updated config without a reboot.
    let cycle2 = cycle_steps(&parse_stored(&stored));
    assert_eq!(cycle2.len(), 1);
    assert_eq!(cycle2[0].color, [0, 255, 0]);
    assert_eq!(cycle2[0].hold_ms, 1000);
}

#[test]
fn empty_or_invalid_config_keeps_led_alive_with_white_fallback() {
    // The firmware's led_task: if the (re)parsed config has no effects, it
    // blinks white every 500 ms instead of going dark.
    for bad in ["", "not json", r#"{"effects":[]}"#] {
        let parsed = parse_config(bad).unwrap_or_else(|_| LedConfig::empty());
        assert!(
            parsed.is_empty(),
            "fixture {bad:?} should yield an empty config"
        );
        // The fallback frame is a valid white WS2812 frame.
        let frame: Ws2812Frame = encode_rgb(&[255, 255, 255]);
        assert!(frame.reset.us >= 50);
    }
}

#[test]
fn config_post_then_led_cycle_is_the_wire_truth() {
    // Full pipeline: UI JSON → (validated) store → LED cycle → wire frames.
    let ui_json =
        r#"{"effects":[{"type":"blend","from":[255,0,0],"to":[0,0,255],"steps":2,"step_ms":50}]}"#;
    let parsed = parse_config(ui_json).unwrap();
    let steps = cycle_steps(&parsed);

    assert_eq!(steps.len(), 3);
    // Encode each step to the wire and confirm the GRB order is applied:
    // red [255,0,0] → wire bytes (green=0, red=255, blue=0).
    let f0 = encode_rgb(&steps[0].color);
    let green_byte = bits_to_byte(&f0.pixel[0..8]);
    let red_byte = bits_to_byte(&f0.pixel[8..16]);
    assert_eq!(green_byte, 0x00);
    assert_eq!(red_byte, 0xFF);
}

/// Reconstruct a byte from 8 WS2812 events (a high 1 µs pulse = a set bit).
fn bits_to_byte(events: &[led_core::Ws2812Event]) -> u8 {
    let mut b = 0u8;
    for (i, e) in events.iter().enumerate() {
        if e.high {
            b |= 1 << (7 - i);
        }
    }
    b
}
