#![no_std]
#![no_main]

//! Web server on ESP32-C5.
//!
//! Serves the LED Effects UI (web/dist/index.html) over HTTP.
//!
//! ⚠️  WiFi credentials must be set before flashing!
//! Edit `WIFI_SSID` and `WIFI_PASS` below.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec::Vec;
use embassy_executor::Spawner;
use embassy_net::{self as net, Config};
use embassy_time::{Duration, Timer};
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::timer::systimer::SystemTimer;
use esp_hal::Async;
use esp_hal::timer::OneShotTimer;
use esp_radio::wifi::{self, Config as WifiConfig, Interface, WifiController};
use esp_rtos;

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// ── Embassy time driver ─────────────────────────────────────────
// embassy-time requires these two symbols to be provided

#[no_mangle]
unsafe extern "C" fn _embassy_time_now() -> u64 {
    esp_hal::time::Instant::now().duration_since_epoch().as_micros()
}

#[no_mangle]
unsafe extern "C" fn _embassy_time_schedule_wake(_t: u64) {
    // embassy-time uses a queue driver, we don't need to implement this
    // for the spin executor (it will poll continuously)
}

// ── WiFi credentials ───────────────────────────────────────────
// ⚠️  CHANGE THESE before flashing!

const WIFI_SSID: &str = "D";
const WIFI_PASS: &str = "REDACTED_WIFI_PASSWORD";

// ── Embedded web page ──────────────────────────────────────────

const INDEX_HTML: &str = include_str!("../web/dist/index.html");

const HTTP_OK_HEADER: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n";
const HTML_404: &[u8] = b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n<html><body><h1>404</h1></body></html>";

// ── LED driver ─────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct LedConfig {
    effects: Vec<LedEffect>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type")]
enum LedEffect {
    #[serde(rename = "blink")]
    Blink { colors: Vec<[u8; 3]>, duration_ms: u32 },
    #[serde(rename = "blend")]
    Blend {
        from: [u8; 3],
        to: [u8; 3],
        steps: u32,
        step_ms: u32,
    },
}

#[inline(always)]
fn ws2812_bit(pin: &mut Output, delay: &Delay, bit: bool) {
    pin.set_high();
    if bit {
        delay.delay_micros(1);
    }
    pin.set_low();
    if !bit {
        delay.delay_micros(1);
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

fn ws2812_rgb(pin: &mut Output, delay: &Delay, rgb: &[u8; 3]) {
    ws2812_grb(pin, delay, rgb[1], rgb[0], rgb[2]);
}

fn interpolate(from: &[u8; 3], to: &[u8; 3], step: u32, total: u32) -> [u8; 3] {
    [
        lerp_u8(from[0], to[0], step, total),
        lerp_u8(from[1], to[1], step, total),
        lerp_u8(from[2], to[2], step, total),
    ]
}

fn lerp_u8(from: u8, to: u8, step: u32, total: u32) -> u8 {
    if total == 0 {
        from
    } else {
        let frac = (step as u32) * 255 / total;
        ((from as u32) * (255 - frac) / 255 + (to as u32) * frac / 255) as u8
    }
}

// ── HTTP server ─────────────────────────────────────────────────

async fn handle_client(socket: &mut embassy_net::tcp::TcpSocket<'static>) {
    let mut buf = [0u8; 512];
    let (mut reader, mut writer) = socket.split();
    
    // Read request
    match reader.read(&mut buf).await {
        Ok(0) => return,
        Ok(n) => {
            let request = core::str::from_utf8(&buf[..n]).unwrap_or("");
            if request.starts_with("GET / ") || request.starts_with("GET /\r\n") {
                let _ = writer.write(HTTP_OK_HEADER).await;
                let _ = writer.write(INDEX_HTML.as_bytes()).await;
            } else {
                let _ = writer.write(HTML_404).await;
            }
        }
        Err(_) => {}
    }
}

// ── LED task ───────────────────────────────────────────────────

const LED_CONFIG_JSON: &str = include_str!("../configs/effects.json");

#[embassy_executor::task]
async fn led_task(mut led: Output<'static>, mut timer: OneShotTimer<'static, Async>) {
    let delay = Delay::new();

    let config: LedConfig = serde_json::from_str(LED_CONFIG_JSON)
        .unwrap_or_else(|_| LedConfig { effects: Vec::new() });

    loop {
        for effect in &config.effects {
            match effect {
                LedEffect::Blink { colors, duration_ms } => {
                    for &color in colors {
                        ws2812_rgb(&mut led, &delay, &color);
                        timer.delay_millis_async(*duration_ms).await;
                    }
                }
                LedEffect::Blend {
                    from,
                    to,
                    steps,
                    step_ms,
                } => {
                    for step in 0..=*steps {
                        let color = interpolate(from, to, step, *steps);
                        ws2812_rgb(&mut led, &delay, &color);
                        timer.delay_millis_async(*step_ms).await;
                    }
                }
            }
        }
    }
}

// ── Main ────────────────────────────────────────────────────────

#[embassy_executor::main(entry = "esp_hal::main")]
async fn main(_spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    // Heap allocator (required by WiFi)
    esp_alloc::heap_allocator!(size: 64 * 1024);

    // Start RTOS scheduler (required for WiFi)
    let timg0 = esp_hal::timer::timg::TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    // Create WiFi station interface (singleton)
    let sta = Interface::station();

    // Initialize WiFi controller
    let mut wifi = WifiController::new(peripherals.WIFI, Default::default())
        .expect("WiFi init failed");

    // Configure station mode with credentials
    let station_config = wifi::sta::StationConfig::default()
        .with_ssid(WIFI_SSID)
        .with_password(WIFI_PASS.to_string());

    wifi.set_config(&WifiConfig::Station(station_config))
        .expect("WiFi config failed");

    // Connect to access point
    wifi.connect_async().await.expect("WiFi connect failed");

    // Initialize networking stack with WiFi interface as driver
    let config = Config::dhcpv4(Default::default());
    let resources: &'static mut embassy_net::StackResources<8> =
        Box::leak(Box::new(embassy_net::StackResources::new()));

    let (stack, runner) = embassy_net::new(
        sta,  // Interface implements embassy-net-driver::Driver
        config,
        resources,
        0,
    );

    // Spawn network runner task
    _spawner.spawn(runner_task(runner)).ok();

    // Wait for DHCP
    while !stack.is_link_up() {
        Timer::after(Duration::from_millis(100)).await;
    }

    // Print IP address
    if let Some(config_v4) = stack.config_v4() {
        esp_println::println!("Connected: {}", config_v4.address);
    }

    // HTTP server loop
    // Buffers must be 'static because the stack is 'static
    static mut RX_BUF: [u8; 1024] = [0; 1024];
    static mut TX_BUF: [u8; 1024] = [0; 1024];
    
    loop {
        let mut socket = unsafe {
            embassy_net::tcp::TcpSocket::new(stack, &mut RX_BUF, &mut TX_BUF)
        };
        let _ = socket.accept(80).await;
        handle_client(&mut socket).await;
    }
}

#[embassy_executor::task]
async fn runner_task(mut runner: embassy_net::Runner<'static, Interface>) {
    runner.run().await;
}