#![no_std]
#![no_main]

//! Web server on ESP32-C5.
//!
//! Serves the LED Effects UI (web/dist/index.html) over HTTP.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec::Vec;
use embassy_net::{Config, Runner, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::Output;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use esp_radio::wifi::{Config as WifiConfig, ControllerConfig, Interface, WifiController, sta::StationConfig};

esp_bootloader_esp_idf::esp_app_desc!();
use esp_backtrace as _;

// ── WiFi credentials ───────────────────────────────────────────
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

// ── HTTP handler ────────────────────────────────────────────────
async fn handle_client(socket: &mut embassy_net::tcp::TcpSocket<'static>) {
    use embedded_io_async::Read;
    let mut buf = [0u8; 512];
    let (mut reader, mut writer) = socket.split();
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

// ── WiFi connection task ───────────────────────────────────────
#[embassy_executor::task]
async fn wifi_task(mut controller: WifiController<'static>) -> ! {
    loop {
        println!("Connecting to WiFi...");
        match controller.connect_async().await {
            Ok(info) => {
                println!("Connected to WiFi: {:?}", info);
                let info = controller.wait_for_disconnect_async().await.ok();
                println!("Disconnected: {:?}", info);
            }
            Err(e) => {
                println!("WiFi connect error: {:?}", e);
            }
        }
        Timer::after(Duration::from_millis(5000)).await;
    }
}

// ── LED effects task ───────────────────────────────────────────
const LED_CONFIG_JSON: &str = include_str!("../configs/effects.json");

#[embassy_executor::task]
async fn led_task() -> ! {
    let delay = Delay::new();

    let config: LedConfig = serde_json::from_str(LED_CONFIG_JSON)
        .unwrap_or_else(|_| LedConfig { effects: Vec::new() });

    loop {
        for effect in &config.effects {
            match effect {
                LedEffect::Blink { colors, duration_ms } => {
                    for &color in colors {
                        Timer::after(Duration::from_millis(*duration_ms as u64)).await;
                    }
                }
                LedEffect::Blend {
                    from,
                    to,
                    steps,
                    step_ms,
                } => {
                    for step in 0..=*steps {
                        Timer::after(Duration::from_millis(*step_ms as u64)).await;
                    }
                }
            }
        }
    }
}

// ── Main ───────────────────────────────────────────────────────
#[no_mangle]
fn main() -> ! {
    // Initialize esp-hal
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Two heap allocators (required by esp-radio)
    esp_alloc::heap_allocator!(size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    // Start RTOS scheduler (required for WiFi)
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    // Configure WiFi station
    let station_config = WifiConfig::Station(
        StationConfig::default()
            .with_ssid(WIFI_SSID)
            .with_password(WIFI_PASS.to_string()),
    );

    // Create WiFi controller with initial config
    let wifi_interface = Interface::station();
    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("WiFi init failed");

    println!("WiFi configured, starting connection task...");

    // Init network stack
    let config = Config::dhcpv4(Default::default());
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        Box::leak(Box::new(StackResources::<3>::new())),
        0,
    );

    // Start embassy executor using esp-rtos thread-mode executor
    let executor = Box::leak(Box::new(esp_rtos::embassy::Executor::new()));
    executor.run(|spawner: embassy_executor::Spawner| {
        // Spawn tasks
        spawner.spawn(wifi_task(controller).expect("spawn wifi_task"));
        spawner.spawn(net_task(runner).expect("spawn net_task"));
        spawner.spawn(http_server_task(stack).expect("spawn http_server_task"));
    });
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await;
}

#[embassy_executor::task]
async fn http_server_task(stack: embassy_net::Stack<'static>) -> ! {
    // Wait for DHCP
    println!("Waiting for DHCP...");
    stack.wait_config_up().await;

    if let Some(config_v4) = stack.config_v4() {
        println!("Got IP: {}", config_v4.address);
    }

    // HTTP server loop
    println!("Starting HTTP server on port 80...");
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