//! Pure, hardware-independent core logic for the ESP32-C5 RGB LED project.
//!
//! This crate has **no hardware dependencies** (only `serde`/`serde_json`),
//! so every piece of logic is unit- and integration-testable on the host
//! (`cargo test -p led-core`):
//!
//! - [`config`] — the effects config model (`configs/effects.json`) and JSON
//!   parsing/validation.
//! - [`effects`] — expands a config into the flat sequence of
//!   (color, hold) steps the LED will emit (one full cycle).
//! - [`color`] — RGB color interpolation (blend effect math).
//! - [`ws2812`] — WS2812 wire-protocol frame encoding (bit order, GRB order,
//!   reset pulse). The LEGACY path ([`ws2812::replay_frame`]) is non-compliant
//!   on real hardware (one un-timed phase per bit); the compliant path is
//!   [`ws2812::replay_frame_v2`] (both timed phases per bit, ns resolution).
//! - [`http`] — minimal HTTP request parsing, routing, and response building
//!   for the on-device web server (GET /, GET /config, POST /config).
//! - [`wifi`] — WiFi credentials config model (single source of truth in
//!   `configs/wifi.json`) and validation.
//!
//! The firmware (root binary + `examples/`) contains the thin hardware layer
//! (GPIO, embassy tasks, WiFi driver) and consumes this crate.

#![no_std]

extern crate alloc;

pub mod color;
pub mod config;
pub mod effects;
pub mod http;
pub mod wifi;
pub mod ws2812;

// Re-export the most used items for convenience.
pub use config::{LedConfig, LedEffect};
pub use ws2812::{encode_grb, encode_rgb, replay_frame, Ws2812Event, Ws2812Frame};
// Corrected, protocol-compliant WS2812 model (two timed phases per bit, ns
// resolution). The legacy items above are non-compliant on real hardware —
// see the module docs in `ws2812.rs`.
pub use ws2812::{encode_grb_v2, encode_rgb_v2, replay_frame_v2, Ws2812BitPhase, Ws2812FrameV2};
