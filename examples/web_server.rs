#![no_std]
#![no_main]

//! Web server on ESP32-C5.
//!
//! Serves the LED Effects UI (`web/dist/index.html`) over HTTP and exposes a
//! two-way config endpoint:
//! - `GET /`          → the embedded UI page
//! - `GET /config`    → the current effect config JSON
//! - `POST /config`   → replace the effect config (validated); the LED task
//!   picks it up live on its next cycle.
//!
//! All pure logic (HTTP parsing/routing, config model, WS2812 encoding, WiFi
//! creds) lives in the `led-core` crate and is unit+integration tested on the
//! host. This file is the thin hardware layer: GPIO bit-banging, embassy
//! tasks, the WiFi driver, and the socket lifecycle.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use embassy_executor::Spawner;
use embassy_net::{Config, Runner, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use esp_radio::wifi::{
    sta::StationConfig, Config as WifiConfig, ControllerConfig, Interface, WifiController,
};

use led_core::config::{LedConfig, LedEffect};
use led_core::http;
use led_core::wifi as led_wifi;
use led_core::ws2812::{self, encode_rgb, PinPulse, Ws2812Frame};

esp_bootloader_esp_idf::esp_app_desc!();
use esp_backtrace as _;

// ── Embedded web page ──────────────────────────────────────────
const INDEX_HTML: &str = include_str!("../web/dist/index.html");
const HTML_404: &[u8] =
    b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n<html><body><h1>404</h1></body></html>";

// ── Shared effect config (live-updatable via POST /config) ─────
// A zeroed byte buffer held as a `static` via interior mutability
// (`UnsafeCell`) — the `mutable_statics`/`static_mut_refs` lints forbid
// `static mut`, and this project denies warnings.
#[repr(transparent)]
struct StaticBuf<const N: usize>(core::cell::UnsafeCell<[u8; N]>);

/// Single-core (ESP32-C5 is one core) + cooperative embassy scheduling: no
/// two tasks ever run at once and no interrupt handler aliases these buffers,
/// so the compiler's `!Sync` default (inherited from `UnsafeCell`) is too
/// conservative — the buffer is soundly shareable.
unsafe impl<const N: usize> Sync for StaticBuf<N> {}

impl<const N: usize> StaticBuf<N> {
    const fn new() -> Self {
        Self(core::cell::UnsafeCell::new([0u8; N]))
    }
    /// Exclusive access. Safety: single-core cooperative scheduling — tasks
    /// never run concurrently and no interrupt handler touches these buffers,
    /// so the caller owns the buffer for the duration of its call.
    // `mut_from_ref` is intentional: `UnsafeCell` exists precisely to permit
    // this documented mutable access from a shared `&self` on a `static`.
    #[allow(clippy::mut_from_ref)]
    fn as_mut(&self) -> &mut [u8; N] {
        // SAFETY: see the method contract above.
        unsafe { &mut *self.0.get() }
    }
    /// A shared `'static` view of the first `len` bytes.
    ///
    /// Sound because the buffer lives in a `static` (valid for the program's
    /// lifetime). Caller must ensure `len <= N` and that no exclusive borrow
    /// via [`as_mut`] is active (single-core cooperative scheduling).
    fn get(&self, len: usize) -> &'static [u8] {
        debug_assert!(len <= N);
        // SAFETY: `self` refers to a `static`; the memory is valid for `len`
        // bytes and no other aliasing access is in flight (see the type docs).
        unsafe { core::slice::from_raw_parts(self.0.get() as *const u8, len) }
    }
}

// Single-core cooperative scheduling: the HTTP task and the LED task never
// run concurrently, so a static buffer guarded by a version counter is safe.
// The LED task re-parses whenever CONFIG_VERSION changes.
const CONFIG_MAX: usize = 4096;
static CONFIG_DATA: StaticBuf<CONFIG_MAX> = StaticBuf::new();
static CONFIG_LEN: AtomicUsize = AtomicUsize::new(0);
static CONFIG_VERSION: AtomicU32 = AtomicU32::new(0);

/// Initialize the shared config with the built-in default (configs/effects.json).
fn config_init() {
    let d = LED_CONFIG_JSON.as_bytes();
    CONFIG_DATA.as_mut()[..d.len()].copy_from_slice(d);
    CONFIG_LEN.store(d.len(), Ordering::SeqCst);
    CONFIG_VERSION.store(0, Ordering::SeqCst);
}

/// Replace the stored config with `new` (already validated). Bumps version.
fn config_set(new: &[u8]) -> bool {
    if new.len() > CONFIG_MAX {
        return false;
    }
    CONFIG_DATA.as_mut()[..new.len()].copy_from_slice(new);
    CONFIG_LEN.store(new.len(), Ordering::SeqCst);
    CONFIG_VERSION.fetch_add(1, Ordering::SeqCst);
    true
}

/// The current stored config as a byte slice (safe: only called between tasks).
fn config_bytes() -> (usize, u32) {
    (
        CONFIG_LEN.load(Ordering::SeqCst),
        CONFIG_VERSION.load(Ordering::SeqCst),
    )
}

// ── WS2812 driver (hardware layer over led-core's encoding) ─────
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
    let frame: Ws2812Frame = encode_rgb(rgb);
    let mut pulse = GpioPulse { pin, delay };
    ws2812::replay_frame(&mut pulse, &frame);
}

// ── HTTP handler ────────────────────────────────────────────────
/// Send a full HTTP response (headers with Content-Length + body) and flush.
///
/// Connection teardown (close + wait + abort) happens in the server loop so a
/// fresh `accept()` is guaranteed to be ready for the next client — the fix
/// for the previous "connection refused on the 2nd request" bug.
async fn send_response(
    socket: &mut embassy_net::tcp::TcpSocket<'static>,
    status: &str,
    content_type: &str,
    body: &[u8],
) {
    let mut header = [0u8; http::RESPONSE_HEADER_BYTES];
    let n = http::build_response_header(&mut header, status, content_type, body.len());
    let _ = socket.write(&header[..n]).await;
    let _ = socket.write(body).await;
    let _ = socket.flush().await;
}

async fn serve_html(socket: &mut embassy_net::tcp::TcpSocket<'static>) {
    println!("HTTP: serving index.html ({} bytes)", INDEX_HTML.len());
    send_response(socket, "200 OK", "text/html", INDEX_HTML.as_bytes()).await;
}

async fn serve_config(socket: &mut embassy_net::tcp::TcpSocket<'static>) {
    let (len, _v) = config_bytes();
    let data = CONFIG_DATA.get(len);
    println!("HTTP: serving current config ({} bytes)", len);
    send_response(socket, "200 OK", "application/json", data).await;
}

async fn handle_post_config(socket: &mut embassy_net::tcp::TcpSocket<'static>, body: &[u8]) {
    let result = http::validate_config_post(body, CONFIG_MAX);
    if result == http::ConfigPostResult::Accepted {
        if config_set(body) {
            println!("HTTP: config updated ({} bytes)", body.len());
        } else {
            println!("HTTP: config too large to store");
        }
    }
    send_response(
        socket,
        http::config_post_status(result),
        "application/json",
        http::config_post_body(result),
    )
    .await;
}

async fn handle_client(socket: &mut embassy_net::tcp::TcpSocket<'static>) {
    // Accumulate the request (headers + body) in a heap Vec so we can keep a
    // reference across `.await` points without holding a `&mut` to a static.
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 512];

    loop {
        if buf.len() >= 4096 {
            break;
        }
        let n = match socket.read(&mut chunk[..]).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                println!("HTTP: read error: {:?}", e);
                return;
            }
        };
        buf.extend_from_slice(&chunk[..n]);
        if http::request_complete(&buf) {
            break;
        }
    }

    if buf.is_empty() {
        return;
    }

    let req = match http::parse_request(&buf) {
        Some(r) => r,
        None => {
            println!("HTTP: incomplete headers");
            return;
        }
    };
    println!(
        "HTTP: {} {} ({} bytes, body {})",
        req.method,
        req.path,
        buf.len(),
        req.content_length
    );

    match http::route(req.method, req.path) {
        http::Route::Index => serve_html(socket).await,
        http::Route::GetConfig => serve_config(socket).await,
        http::Route::PostConfig => handle_post_config(socket, req.body).await,
        http::Route::NotFound => {
            println!("HTTP: 404 for {} {}", req.method, req.path);
            send_response(socket, "404 Not Found", "text/html", HTML_404).await;
        }
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

    let mut config: LedConfig =
        serde_json::from_str(LED_CONFIG_JSON).unwrap_or_else(|_| LedConfig::empty());
    let mut loaded_version = CONFIG_VERSION.load(Ordering::SeqCst);
    println!(
        "LED: {} effects loaded (v{})",
        config.effects.len(),
        loaded_version
    );

    loop {
        // Reload the config if it changed (via POST /config).
        let version = CONFIG_VERSION.load(Ordering::SeqCst);
        if version != loaded_version {
            let (len, _) = config_bytes();
            let data = CONFIG_DATA.get(len);
            match serde_json::from_slice::<LedConfig>(data) {
                Ok(c) if !c.is_empty() => {
                    config = c;
                    loaded_version = version;
                    println!(
                        "LED: config reloaded ({} effects, v{})",
                        config.effects.len(),
                        version
                    );
                }
                _ => {
                    loaded_version = version;
                    println!("LED: config reload failed, keeping previous");
                }
            }
        }

        if config.is_empty() {
            // No valid config: blink white so the LED is clearly alive.
            ws2812_rgb(&mut pin, &delay, &[255, 255, 255]);
            Timer::after(Duration::from_millis(500)).await;
            continue;
        }

        for effect in &config.effects {
            match effect {
                LedEffect::Blink {
                    colors,
                    duration_ms,
                } => {
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
                    ..
                } => {
                    for step in 0..=*steps {
                        let color = led_core::color::interpolate(from, to, step, *steps);
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

    // Two heap allocators (required by esp-radio and by serde_json parsing
    // below — the heap MUST exist before any allocation).
    esp_alloc::heap_allocator!(size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    // Load + validate WiFi credentials from the single source of truth
    // (configs/wifi.json) BEFORE touching the WiFi driver.
    let creds = led_wifi::default_wifi_config().expect("wifi.json failed to parse");
    if let Err(e) = creds.validate() {
        println!("!!! WiFi credentials invalid: {e:?} — check configs/wifi.json");
        loop {
            Timer::after(Duration::from_secs(60)).await;
        }
    }

    // Start RTOS scheduler (required for WiFi)
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    // Create the WS2812 LED pin (GPIO27)
    let led_pin = Output::new(peripherals.GPIO27, Level::Low, OutputConfig::default());

    // Initialize the shared effect config with the built-in default.
    config_init();

    // Configure WiFi station (credentials from configs/wifi.json).
    let station_config = WifiConfig::Station(
        StationConfig::default()
            .with_ssid(creds.ssid.as_str())
            .with_password(creds.password),
    );

    // Create WiFi controller with initial config
    let wifi_interface = Interface::station();
    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("WiFi init failed");

    println!(
        "WiFi configured (ssid=\"{}\"), spawning tasks...",
        creds.ssid
    );

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
    static RX_BUF: StaticBuf<1024> = StaticBuf::new();
    // TX buffer must be large enough to hold the whole HTML page (~16 KB) so a
    // single write + flush can deliver it.
    static TX_BUF: StaticBuf<16384> = StaticBuf::new();

    loop {
        // SAFETY: this task owns the buffers for the whole accept/handle cycle
        // (single connection at a time; no concurrent access).
        let mut socket = embassy_net::tcp::TcpSocket::new(stack, RX_BUF.as_mut(), TX_BUF.as_mut());
        let _ = socket.accept(80).await;
        handle_client(&mut socket).await;
        // Full teardown so the next accept() is ready. This matches the
        // esp-hal web-server reference exactly: flush (already done in
        // send_response) → close the write half (FIN) → wait for the TCP
        // stack to process the connection → force-close both halves (RST).
        // The ~1 s wait is required: re-accepting sooner races the stack's
        // teardown and the next connect() is refused.
        socket.close();
        Timer::after(Duration::from_millis(1000)).await;
        socket.abort();
    }
}
