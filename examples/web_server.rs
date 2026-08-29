#![no_std]
#![no_main]

//! Web server on ESP32-C5.
//!
//! Serves the LED Effects UI (web/dist/index.html) over HTTP.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec::Vec;
use embassy_executor::Spawner;
use embassy_net::{Config, Runner, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
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
const HTML_404: &[u8] = b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n<html><body><h1>404</h1></body></html>";

/// Write `n` as decimal ASCII into `buf`; returns the number of bytes written.
fn write_uint(buf: &mut [u8], n: usize) -> usize {
    if n == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut digits = [0u8; 12];
    let mut i = digits.len();
    let mut v = n;
    while v > 0 {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let len = digits.len() - i;
    buf[..len].copy_from_slice(&digits[i..]);
    len
}

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
    let mut buf = [0u8; 512];
    let n = match socket.read(&mut buf).await {
        Ok(0) => {
            println!("HTTP: client closed before request");
            return;
        }
        Ok(n) => n,
        Err(e) => {
            println!("HTTP: read error: {:?}", e);
            return;
        }
    };
    let request = core::str::from_utf8(&buf[..n]).unwrap_or("");
    println!("HTTP: request ({} bytes): {:?}", n, &request[..n.min(40)]);

    if request.starts_with("GET /") {
        let html = INDEX_HTML.as_bytes();
        let mut header = [0u8; 128];
        let mut pos = 0;
        let prefix = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: ";
        header[pos..pos + prefix.len()].copy_from_slice(prefix);
        pos += prefix.len();
        pos += write_uint(&mut header[pos..], html.len());
        let suffix = b"\r\nConnection: close\r\n\r\n";
        header[pos..pos + suffix.len()].copy_from_slice(suffix);
        pos += suffix.len();
        println!("HTTP: serving {} bytes (body {})", pos + html.len(), html.len());
        let _ = socket.write(&header[..pos]).await;
        let _ = socket.write(html).await;
        let _ = socket.flush().await;
        socket.close();
        println!("HTTP: response sent + flushed + closed");
    } else {
        println!("HTTP: 404 for: {:?}", request);
        let _ = socket.write(HTML_404).await;
        let _ = socket.flush().await;
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
async fn led_task(led: LedPin) -> ! {
    let mut pin = led.0;
    let delay = Delay::new();

    let config: LedConfig = serde_json::from_str(LED_CONFIG_JSON)
        .unwrap_or_else(|_| LedConfig { effects: Vec::new() });

    loop {
        for effect in &config.effects {
            match effect {
                LedEffect::Blink { colors, duration_ms } => {
                    for &color in colors {
                        ws2812_rgb(&mut pin, &delay, &color);
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
                        let color = interpolate(from, to, step, *steps);
                        ws2812_rgb(&mut pin, &delay, &color);
                        Timer::after(Duration::from_millis(*step_ms as u64)).await;
                    }
                }
            }
        }
    }
}

// ── Main ───────────────────────────────────────────────────────
// The GPIO pin handle is !Send (PhantomData<&'lt mut ()>). It is safe to move
// it into a task on this single-core target because only led_task uses it.
struct LedPin(Output<'static>);
unsafe impl Send for LedPin {}

#[esp_hal::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

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

    // Create the WS2812 LED pin (GPIO27)
    let led_pin = Output::new(peripherals.GPIO27, Level::Low, OutputConfig::default());

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

    println!("WiFi configured, spawning tasks...");

    // Init network stack
    let net_config = Config::dhcpv4(Default::default());
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        net_config,
        Box::leak(Box::new(StackResources::<3>::new())),
        0,
    );

    // Spawn all tasks
    spawner.spawn(wifi_task(controller).expect("spawn wifi_task"));
    spawner.spawn(net_task(runner).expect("spawn net_task"));
    spawner.spawn(http_server_task(stack).expect("spawn http_server_task"));
    spawner.spawn(led_task(LedPin(led_pin)).expect("spawn led_task"));

    // Keep the main task alive
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
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
    // TX buffer must be large enough to hold the whole HTML page (14 KB) so a
    // single write + flush can deliver it.
    static mut TX_BUF: [u8; 16384] = [0; 16384];

    loop {
        let mut socket = unsafe {
            embassy_net::tcp::TcpSocket::new(stack, &mut RX_BUF, &mut TX_BUF)
        };
        let _ = socket.accept(80).await;
        handle_client(&mut socket).await;
    }
}