//! Config-driven LED effects (sync/blocking).
//!
//! Reads a JSON config from `configs/effects.json` and runs the specified
//! effects (blink and blend) in a loop.
//!
//! The config is embedded at compile time using `include_str!`.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// ── Config structs ──────────────────────────────────────────────────

/// Top-level config.
#[derive(serde::Deserialize)]
struct Config {
    effects: Vec<Effect>,
}

/// An effect to run.
#[derive(serde::Deserialize)]
#[serde(tag = "type")]
enum Effect {
    #[serde(rename = "blink")]
    Blink {
        colors: Vec<[u8; 3]>,
        duration_ms: u32,
    },
    #[serde(rename = "blend")]
    Blend {
        from: [u8; 3],
        to: [u8; 3],
        steps: u32,
        step_ms: u32,
    },
}

// ── WS2812 driver ──────────────────────────────────────────────────

/// WS2812 bitbang driver for a single addressable RGB LED.
///
/// Protocol timing (WS2812):
///   - Logic 0: HIGH < 0.5 µs, LOW > 0.5 µs
///   - Logic 1: HIGH > 0.5 µs, LOW < 0.5 µs
///   - Reset:   LOW ≥ 50 µs
///
/// Data order: GRB (Green, Red, Blue) — standard for WS2812 LEDs.

/// Send one WS2812 bit via GPIO with calibrated delays.
#[inline(always)]
fn ws2812_bit(pin: &mut Output, delay: &Delay, bit: bool) {
    pin.set_high();
    if bit {
        delay.delay_micros(1); // HIGH ~1 µs → logic 1
    }
    pin.set_low();
    if !bit {
        delay.delay_micros(1); // LOW ~1 µs → logic 0
    }
}

/// Send one byte (MSB first) to the WS2812.
#[inline(always)]
fn ws2812_byte(pin: &mut Output, delay: &Delay, byte: u8) {
    for i in (0..8).rev() {
        ws2812_bit(pin, delay, (byte >> i) & 1 != 0);
    }
}

/// Send a GRB triplet to the WS2812 and issue a reset pulse.
fn ws2812_grb(pin: &mut Output, delay: &Delay, g: u8, r: u8, b: u8) {
    ws2812_byte(pin, delay, g);
    ws2812_byte(pin, delay, r);
    ws2812_byte(pin, delay, b);
    pin.set_low();
    delay.delay_micros(50);
}

/// Send an RGB color (as [R, G, B]) to the WS2812.
fn ws2812_rgb(pin: &mut Output, delay: &Delay, rgb: &[u8; 3]) {
    ws2812_grb(pin, delay, rgb[1], rgb[0], rgb[2]); // RGB → GRB
}

// ── Color interpolation ────────────────────────────────────────────

/// Linearly interpolate between two RGB colors.
fn interpolate(from: &[u8; 3], to: &[u8; 3], step: u32, total: u32) -> [u8; 3] {
    [
        lerp_u8(from[0], to[0], step, total),
        lerp_u8(from[1], to[1], step, total),
        lerp_u8(from[2], to[2], step, total),
    ]
}

/// Linear interpolation for a single u8 channel.
fn lerp_u8(from: u8, to: u8, step: u32, total: u32) -> u8 {
    if total == 0 {
        from
    } else {
        let frac = (step as u32) * 255 / total;
        let result = (from as u32) * (255 - frac) / 255 + (to as u32) * frac / 255;
        result as u8
    }
}

// ── Main ────────────────────────────────────────────────────────────

const CONFIG_JSON: &str = include_str!("../configs/effects.json");

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    // Initialize heap allocator for serde_json / alloc
    esp_alloc::heap_allocator!(size: 8192);

    let delay = Delay::new();

    let config = OutputConfig::default();
    let mut led = Output::new(peripherals.GPIO27, Level::Low, config);

    // Parse config at startup
    let config: Config = serde_json::from_str(CONFIG_JSON)
        .unwrap_or_else(|_| {
            // Fallback: empty config
            Config { effects: Vec::new() }
        });

    // Run effects in a loop
    loop {
        for effect in &config.effects {
            match effect {
                Effect::Blink { colors, duration_ms } => {
                    for &color in colors {
                        ws2812_rgb(&mut led, &delay, &color);
                        delay.delay_millis(*duration_ms as u32);
                    }
                }
                Effect::Blend {
                    from,
                    to,
                    steps,
                    step_ms,
                } => {
                    for step in 0..=*steps {
                        let color = interpolate(from, to, step, *steps);
                        ws2812_rgb(&mut led, &delay, &color);
                        delay.delay_millis(*step_ms as u32);
                    }
                }
            }
        }
    }
}