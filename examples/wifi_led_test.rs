#![no_std]
#![no_main]

//! Phase-colored WiFi init test.
//! Each phase writes a REAL WS2812 color frame (with 50us reset) so the
//! LED reliably shows exactly how far the firmware got.
//!
//!   RED        = firmware started (main entry)
//!   BLUE       = heap allocated
//!   YELLOW     = esp_rtos started
//!   MAGENTA    = WiFi station config ready
//!   WHITE      = right before WifiController::new()
//!   GREEN      = WifiController::new() OK
//!   CYAN       = connecting (about to call connect_async in async version)
//!   FLASH RED  = WiFi init failed

extern crate alloc;

use alloc::string::ToString;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    interrupt::software::SoftwareInterruptControl,
    ram,
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{
    Config as WifiConfig,
    ControllerConfig,
    WifiController,
    WifiError,
    sta::StationConfig,
};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();
use esp_backtrace as _;

const WIFI_SSID: &str = "D";
const WIFI_PASS: &str = "REDACTED_WIFI_PASSWORD";

// ── WS2812 bitbang driver (same as led_effects_async) ─────────────

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

#[inline(always)]
fn ws2812_byte(pin: &mut Output, delay: &Delay, byte: u8) {
    for i in (0..8).rev() {
        ws2812_bit(pin, delay, (byte >> i) & 1 != 0);
    }
}

fn ws2812_grb(pin: &mut Output, delay: &Delay, g: u8, r: u8, b: u8) {
    ws2812_byte(pin, delay, g);
    ws2812_byte(pin, delay, r);
    ws2812_byte(pin, delay, b);
    pin.set_low();
    delay.delay_micros(50);
}

/// Set a solid color (RGB array).
fn set_color(led: &mut Output, delay: &Delay, r: u8, g: u8, b: u8) {
    ws2812_grb(led, delay, g, r, b);
}

// ── Main ───────────────────────────────────────────────────────────

#[esp_hal::main]
fn main() -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let mut led = Output::new(peripherals.GPIO27, Level::Low, OutputConfig::default());
    let delay = Delay::new();

    println!("=== WiFi phase test ===");

    // RED = started
    set_color(&mut led, &delay, 255, 0, 0);
    println!("[1] started, setting RED");

    // BLUE = heap
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);
    set_color(&mut led, &delay, 0, 0, 255);
    println!("[2] heap allocated, setting BLUE");

    // YELLOW = RTOS started
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);
    set_color(&mut led, &delay, 255, 255, 0);

    // MAGENTA = WiFi station config ready
    let station_config = WifiConfig::Station(
        StationConfig::default()
            .with_ssid(WIFI_SSID)
            .with_password(WIFI_PASS.to_string()),
    );
    set_color(&mut led, &delay, 255, 0, 255);

    // WHITE = right before WifiController::new()
    set_color(&mut led, &delay, 255, 255, 255);
    println!("[5] WHITE set, about to call WifiController::new()");

    let controller = match WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    ) {
        Ok(c) => c,
        Err(e) => {
            println!("!!! WiFi init FAILED: {:?}", e);
            // Blink N times where N = error code, then pause, repeat.
            //   1 = Unsupported
            //   2 = InvalidArguments
            //   3 = Failed (ESP_FAIL from blob)
            //   4 = OutOfMemory
            //   5 = InvalidSsid
            //   6 = InvalidPassword
            //   7 = NotConnected
            let code: u32 = match e {
                WifiError::Unsupported => 1,
                WifiError::InvalidArguments => 2,
                WifiError::Failed => 3,
                WifiError::OutOfMemory => 4,
                WifiError::InvalidSsid => 5,
                WifiError::InvalidPassword => 6,
                WifiError::NotConnected => 7,
                _ => 8,
            };
            loop {
                for _ in 0..code {
                    set_color(&mut led, &delay, 255, 0, 0);
                    delay.delay_millis(80);
                    set_color(&mut led, &delay, 0, 0, 0);
                    delay.delay_millis(80);
                }
                delay.delay_millis(1500);
            }
        }
    };

    // GREEN = WifiController::new() OK
    set_color(&mut led, &delay, 0, 255, 0);
    println!("[6] WifiController::new() OK, setting GREEN");

    // Stay green (WiFi driver is initialized and idle)
    loop {
        delay.delay_millis(10_000);
    }
}