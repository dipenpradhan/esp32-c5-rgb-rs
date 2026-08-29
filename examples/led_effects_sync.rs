//! Config-driven LED effects (sync/blocking example).
//!
//! Reads a JSON config from `configs/effects.json` and runs the specified
//! effects (blink and blend) in a loop. The config is embedded at compile
//! time using `include_str!`.
//!
//! All logic (config model, WS2812 encoding/replay, color interpolation)
//! lives in the `led-core` crate and is unit+integration tested on the host;
//! this example is the thin hardware layer.

#![no_std]
#![no_main]

extern crate alloc;

use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
use led_core::config::{LedConfig, LedEffect};
use led_core::ws2812::{self, encode_rgb, PinPulse};

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// ── WS2812 driver (hardware layer over led-core's encoding) ──────────────

/// A `led_core::ws2812::PinPulse` backed by a real GPIO pin + calibrated delay.
struct GpioPulse<'a> {
    pin: &'a mut Output<'static>,
    delay: &'a Delay,
}

impl PinPulse for GpioPulse<'_> {
    fn set_high(&mut self) {
        self.pin.set_high();
    }
    fn set_low(&mut self) {
        self.pin.set_low();
    }
    fn delay_us(&mut self, us: u32) {
        self.delay.delay_micros(us);
    }
}

/// Send one RGB color (`[R, G, B]`) to the WS2812 via the tested `led-core`
/// frame encoding + replay (24 µs bit-bang + 50 µs reset = 74 µs).
fn ws2812_rgb(pin: &mut Output<'static>, delay: &Delay, rgb: &[u8; 3]) {
    let frame = encode_rgb(rgb);
    let mut pulse = GpioPulse { pin, delay };
    ws2812::replay_frame(&mut pulse, &frame);
}

// ── Main ──────────────────────────────────────────────────────────────────

const CONFIG_JSON: &str = include_str!("../configs/effects.json");

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    // Initialize heap allocator for serde_json / alloc
    esp_alloc::heap_allocator!(size: 8192);

    let delay = Delay::new();

    let config = OutputConfig::default();
    let mut led = Output::new(peripherals.GPIO27, Level::Low, config);

    // Parse config at startup (falls back to an empty config if malformed).
    let config: LedConfig =
        serde_json::from_str(CONFIG_JSON).unwrap_or_else(|_| LedConfig::empty());

    // Run effects in a loop
    loop {
        for effect in &config.effects {
            match effect {
                LedEffect::Blink {
                    colors,
                    duration_ms,
                } => {
                    for &color in colors {
                        ws2812_rgb(&mut led, &delay, &color);
                        delay.delay_millis(*duration_ms);
                    }
                }
                LedEffect::Blend {
                    from,
                    to,
                    steps,
                    step_ms,
                    ..
                } => {
                    for step in 0..=*steps {
                        let color = led_core::color::interpolate(from, to, step, *steps);
                        ws2812_rgb(&mut led, &delay, &color);
                        delay.delay_millis(*step_ms);
                    }
                }
            }
        }
    }
}
