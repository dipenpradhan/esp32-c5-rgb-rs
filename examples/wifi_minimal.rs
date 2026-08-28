#![no_std]
#![no_main]

//! Minimal WiFi test - just init WiFi and report via LED

extern crate alloc;

use alloc::string::ToString;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    timer::timg::TimerGroup,
};
use esp_radio::wifi::{
    Config as WifiConfig,
    ControllerConfig,
    WifiController,
    sta::StationConfig,
};

esp_bootloader_esp_idf::esp_app_desc!();
use esp_backtrace as _;

const WIFI_SSID: &str = "D";
const WIFI_PASS: &str = "REDACTED_WIFI_PASSWORD";

#[esp_hal::main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // LED = GPIO27
    let mut led = Output::new(peripherals.GPIO27, Level::Low, OutputConfig::default());
    let delay = Delay::new();

    // Blink 3x = startup OK
    for _ in 0..3 {
        led.set_high();
        delay.delay_millis(150);
        led.set_low();
        delay.delay_millis(150);
    }

    // Init heap (required by esp-radio)
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    // Start RTOS
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    // LED solid = heap + RTOS OK
    led.set_high();
    delay.delay_millis(500);

    // WiFi init
    led.toggle();
    delay.delay_millis(200);

    let station_config = WifiConfig::Station(
        StationConfig::default()
            .with_ssid(WIFI_SSID)
            .with_password(WIFI_PASS.to_string()),
    );

    let controller = match WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    ) {
        Ok(c) => c,
        Err(_) => {
            // Fast blink 5x = WiFi init failed
            for _ in 0..5 {
                led.toggle();
                delay.delay_millis(100);
            }
            loop { delay.delay_millis(1000); }
        }
    };

    // LED solid = WiFi controller created
    led.set_high();
    delay.delay_millis(500);

    // Try connect (blocking)
    led.toggle();
    delay.delay_millis(200);

    match controller.connect() {
        Ok(info) => {
            // Slow blink = connected
            println!("Connected: {:?}", info);
            loop {
                led.toggle();
                delay.delay_millis(500);
            }
        }
        Err(_) => {
            // Fast blink 10x = connect failed
            for _ in 0..10 {
                led.toggle();
                delay.delay_millis(80);
            }
            loop { delay.delay_millis(1000); }
        }
    }
}