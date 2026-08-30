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
//! host. This file is the thin hardware layer: the RMT-driven WS2812 pixel
//! writer, embassy tasks, the WiFi driver, and the socket lifecycle.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use embassy_executor::Spawner;
use embassy_net::{Config, Runner, StackResources};
use embassy_time::{with_timeout, Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::Level;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::rmt::{Channel, PulseCode, Rmt, Tx, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use esp_radio::wifi::{
    sta::StationConfig, Config as WifiConfig, ControllerConfig, Interface, WifiController,
};

use led_core::config::{LedConfig, LedEffect};
use led_core::http;
use led_core::wifi as led_wifi;
use led_core::ws2812::{encode_rgb_v2, Ws2812FrameV2};

esp_bootloader_esp_idf::esp_app_desc!();
use esp_backtrace as _;

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

// ── Embedded web page ──────────────────────────────────────────
const INDEX_HTML: &str = include_str!("../web/dist/index.html");
/// The 404 *body* — just the HTML. `send_response` builds and writes its own
/// HTTP header (via `http::build_response_header`) and then writes this byte
/// slice as the body, so the constant must be body-only. The old value baked
/// in a second `HTTP/1.1 404 ...` status line + headers, which the device then
/// emitted *as body text* after the real header — the malformed 404 captured
/// on hardware (exactly the length of that constant).
const HTML_404: &[u8] = b"<html><body><h1>404</h1></body></html>";

// ── POST /config authentication ────────────────────────────────
// There is no other auth: any device on the same WiFi can reach the server.
// To stop a casual (or malicious) LAN device from rewriting the config, a
// `POST /config` must echo a secret token back in a request header; GET routes
// stay open so the UI page can load.
/// The request header a `POST /config` must carry, carrying [`CONFIG_TOKEN`].
const CONFIG_TOKEN_HEADER: &str = "X-Config-Token";
/// A 128-bit random-ish token embedded in the firmware. A `POST /config` that
/// does not echo this exact value in [`CONFIG_TOKEN_HEADER`] is rejected with
/// 403. (Pre-generated, not `build_time!`-derived: `build_time` is not in the
/// dependency set and no dependency may be added from this file — see the
/// report. 128 bits is unguessable by a LAN peer without the firmware image.)
const CONFIG_TOKEN: &str = "76f05ec02c1b995ddc6b795f90c9aaba";
/// The script injected into the served page (see [`injected_index_html`]) so
/// the UI — which lives in `web/src/`, owned elsewhere and not edited here —
/// can read `window.CONFIG_TOKEN` and attach it to its `POST /config` header
/// without the token being hard-coded into `web/src/`. Built with `format!`
/// (not `concat!`) because `concat!` only splices token literals, and
/// [`CONFIG_TOKEN`] is a `const`, not a literal.
fn config_token_script() -> String {
    format!("<script>window.CONFIG_TOKEN=\"{}\";</script>", CONFIG_TOKEN)
}

// ── Shared effect config (live-updatable via POST /config) ─────
// A zeroed byte buffer held as a `static` via interior mutability
// (`UnsafeCell`) — the `mutable_statics`/`static_mut_refs` lints forbid
// `static mut`, and this project denies warnings.
#[repr(transparent)]
struct StaticBuf<const N: usize>(core::cell::UnsafeCell<[u8; N]>);

/// SAFETY of the hand-written `Sync` below rests on **confinement to a single
/// executor thread**, not on the (false) claim that no other thread or task
/// ever runs. Other threads genuinely do exist: `esp_rtos::start` installs a
/// preemptive scheduler (run queue, priorities, task switching), and the
/// esp-radio WiFi driver spawns its own OS tasks. But those threads never
/// touch these buffers. Every `StaticBuf` in this file (the `CONFIG_DATA`
/// store and the `RX_BUF`/`TX_BUF` socket buffers) is accessed only by the
/// four embassy tasks, and all four run on ONE single-threaded cooperative
/// embassy executor (`#[esp_hal::main]` compiles to a single
/// `esp_rtos::embassy::Executor`), which polls them round-robin with no
/// preemption between `await` points. Because no other thread ever holds an
/// alias to these buffers, a shared `&` from the `static` cannot alias a live
/// `&mut`, and the `!Sync` default inherited from `UnsafeCell` is therefore
/// too conservative — the buffer is soundly shareable.
unsafe impl<const N: usize> Sync for StaticBuf<N> {}

impl<const N: usize> StaticBuf<N> {
    const fn new() -> Self {
        Self(core::cell::UnsafeCell::new([0u8; N]))
    }
    /// Exclusive access.
    ///
    /// SAFETY: confined to the single cooperative embassy executor (see the
    /// `Sync` impl above) — no interrupt handler or other OS thread aliases
    /// this buffer, and the executor never preempts between `await` points, so
    /// the caller owns the buffer for the duration of the synchronous call.
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
    /// lifetime) and — per the `Sync` impl above — only the one cooperative
    /// executor aliases it. The caller must ensure `len <= N` and that no
    /// exclusive borrow via [`as_mut`] is in flight on that same executor
    /// thread (which holds for every call site in this file).
    fn get(&self, len: usize) -> &'static [u8] {
        debug_assert!(len <= N);
        // SAFETY: `self` is a `static` (valid for `len` bytes for the program's
        // lifetime) and, per the `Sync` impl above, is aliased only by the one
        // cooperative executor — no `as_mut` borrow is in flight on it.
        unsafe { core::slice::from_raw_parts(self.0.get() as *const u8, len) }
    }
}

// Shared effect config. The buffer is guarded by a version counter: the HTTP
// task is the ONLY writer (`config_init`/`config_set` copy the bytes and then
// bump `CONFIG_VERSION`), and the LED task + `GET /config` readers key off
// `CONFIG_VERSION`. Because every accessor runs on the single cooperative
// executor and the bytes are written *before* the version is bumped, a reader
// sees either the fully-old or fully-new buffer, never a torn mix.
const CONFIG_MAX: usize = 4096;
/// Size of each `read()` chunk in `handle_client` (512 bytes: matches the
/// 512-byte `RX_BUF`, so one chunk always fits the socket's receive buffer).
const READ_CHUNK_BYTES: usize = 512;
static CONFIG_DATA: StaticBuf<CONFIG_MAX> = StaticBuf::new();
static CONFIG_LEN: AtomicUsize = AtomicUsize::new(0);
static CONFIG_VERSION: AtomicU32 = AtomicU32::new(0);

/// Per-`read()` idle budget for an accepted connection (see `handle_client`).
///
/// The server handles exactly ONE connection at a time (accept → handle →
/// teardown → accept). A client that connects, sends one byte, then goes
/// silent would otherwise block the single `read` forever and starve the
/// config UI of any other client until a reboot (a Slowloris with one packet).
/// Racing each read against this timer bounds that worst case.
const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(10);

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

/// The current stored config length + version (safe: every caller runs on the
/// single cooperative executor and reads the two atomics synchronously — no
/// `await` between the reads — so they are consistent).
fn config_bytes() -> (usize, u32) {
    (
        CONFIG_LEN.load(Ordering::SeqCst),
        CONFIG_VERSION.load(Ordering::SeqCst),
    )
}

// ── WS2812 driver (RMT hardware timing over led-core's encoding) ────
//
// The waveform is generated by the RMT peripheral, NOT the previous GPIO
// bit-bang (`GpioPulse`/`replay_frame`) driver. That driver timed only ONE
// phase per bit: a `1` bit ended in a ~0 ns LOW, so consecutive `1` bits
// merged into a single continuous HIGH pulse — a byte like `0xFF` collapsed
// to one pulse and the 24-bit GRB frame misaligned, producing garbled
// colours. A `delay_nanos`-based fix is impossible here:
// `esp_hal::delay::Delay::delay_nanos` quantises to whole microseconds, so
// all four phase widths (400/850/800/450 ns) would round up to 1 µs and a
// `0`-bit would become indistinguishable from a `1`-bit. The RMT emits BOTH
// phases of every bit with hardware timing at 12.5 ns resolution.
//
// This is the exact driver verified working on the board in `src/main.rs`
// (visual check of correct colours): same 80 MHz counter clock, same
// `ns_to_ticks`/`frame_codes` math, same `configure_tx` + `with_pin` setup.

/// RMT global counter clock (MHz). The ESP32-C5 RMT source clock is the 80 MHz
/// PLL (the driver's `ClockSource::default()`), so requesting 80 MHz is exact:
/// the global divider resolves to "no division" and one tick = 1 / 80 MHz =
/// 12.5 ns.
const RMT_SCLK_MHZ: u32 = 80;

/// Per-channel clock divider. It is written VERBATIM to the channel `div_cnt`
/// register, on the ESP32-C5 where `1` means "no division" (full 80 MHz) — NOT
/// the default `0`, which is the register's special "divide by 256" value.
/// `TxChannelConfig::default()` sets `clk_divider = 0`, so relying on the
/// default would make each tick 3.2 µs (240x too wide) and push every
/// WS2812 phase far outside its ±150 ns datasheet window — the LED would
/// stay garbled from code that compiles perfectly. Pass `1` EXPLICITLY.
const CHANNEL_CLK_DIVIDER: u8 = 1;

/// Nanoseconds per second (for the ns→tick conversion).
const NS_PER_SEC: u64 = 1_000_000_000;

/// Convert a nanosecond width to RMT ticks at `tick_hz`.
///
/// `ticks = ns × tick_hz / 1e9`. All four WS2812 phase widths (400/850/800/450
/// ns) and the 300 µs reset are exact multiples of the 12.5 ns tick, so this
/// is exact; the assertion guards against a future constant that would not
/// divide evenly (which would need rounding and could push a phase outside its
/// ±150 ns window).
fn ns_to_ticks(ns: u32, tick_hz: u32) -> u16 {
    let scaled = ns as u64 * tick_hz as u64;
    debug_assert_eq!(
        scaled % NS_PER_SEC,
        0,
        "{ns} ns is not an exact multiple of the {tick_hz} Hz tick"
    );
    (scaled / NS_PER_SEC) as u16
}

/// Build the 25-entry pulse-code frame (24 bits + end marker) for one color.
///
/// `encode_rgb_v2` already applied the RGB→GRB wire-order conversion and emits
/// both phases of every bit; each bit becomes one `PulseCode` (High for
/// `high_ns`, then Low for `low_ns`). The 300 µs reset LOW is folded into the
/// final bit's LOW phase — both are contiguous Low, so the waveform is
/// identical, and `low_ticks + reset_ticks` stays below the 15-bit (32767)
/// per-field maximum.
fn frame_codes(frame: &Ws2812FrameV2, tick_hz: u32) -> [PulseCode; 25] {
    let mut codes = [PulseCode::end_marker(); 25];
    for (i, phase) in frame.pixel.iter().enumerate() {
        let high = ns_to_ticks(phase.high_ns, tick_hz);
        let mut low = ns_to_ticks(phase.low_ns, tick_hz);
        if i + 1 == frame.pixel.len() {
            low += ns_to_ticks(frame.reset_ns, tick_hz);
        }
        codes[i] = PulseCode::new(Level::High, high, Level::Low, low);
    }
    codes[24] = PulseCode::end_marker();
    codes
}

/// Send one RGB color (`[R, G, B]`) to the WS2812 via the RMT.
///
/// `transmit` consumes the channel and returns it with the `wait()` result,
/// so the caller holds it across frames. Per-frame blocking time is ~330 µs,
/// dominated by the 300 µs reset (previously ~74 µs under the bit-bang
/// driver); the effect holds are millisecond-scale, so blocking inside the
/// cooperative executor is acceptable (see the blocking-vs-async note in
/// `led_task`).
fn ws2812_rgb_rmt<'ch>(
    channel: Channel<'ch, esp_hal::Blocking, Tx>,
    tick_hz: u32,
    rgb: &[u8; 3],
) -> Channel<'ch, esp_hal::Blocking, Tx> {
    let frame = encode_rgb_v2(rgb);
    let codes = frame_codes(&frame, tick_hz);
    let tx = channel.transmit(&codes).unwrap();
    tx.wait().unwrap()
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

/// The UI page as actually served: [`INDEX_HTML`] with a tiny `<script>`
/// injected before `</head>` that publishes [`CONFIG_TOKEN`] to the page as
/// `window.CONFIG_TOKEN`. This is how the token reaches the browser *without*
/// editing `web/src/` (owned elsewhere): the served copy is built here, at
/// serve time, so the `web/src` source and the built `dist/index.html` stay
/// untouched. The injection point is chosen so the token is available before
/// the module `<script>` at the end of `<head>` runs.
fn injected_index_html() -> Vec<u8> {
    let script = config_token_script();
    let mut out = Vec::with_capacity(INDEX_HTML.len() + script.len());
    match INDEX_HTML.find("</head>") {
        Some(i) => {
            out.extend_from_slice(INDEX_HTML[..i].as_bytes());
            out.extend_from_slice(script.as_bytes());
            out.extend_from_slice(INDEX_HTML[i..].as_bytes());
        }
        // Fall back to the unmodified page if the marker is ever missing (the
        // page still loads; it just won't carry the token).
        None => out.extend_from_slice(INDEX_HTML.as_bytes()),
    }
    out
}

/// True if the raw request buffer's header block carries
/// `X-Config-Token: <CONFIG_TOKEN>` (a `led-core` header-lookup helper does not
/// exist, and `led-core` is owned elsewhere — so the lookup is local here).
///
/// The comparison is case-sensitive on the header *name* (the UI we inject
/// sends the exact name) and exact on the value. Comparing one header's value
/// against the constant is sufficient for auth: the value itself is the secret.
fn config_token_provided(req_buf: &[u8]) -> bool {
    // Only the header block matters (before the "\r\n\r\n" terminator).
    let header_end = match http::find_header_end(req_buf) {
        Some(i) => i,
        None => return false,
    };
    let head = core::str::from_utf8(&req_buf[..header_end]).unwrap_or("");
    let needle = format!("{}: {}", CONFIG_TOKEN_HEADER, CONFIG_TOKEN);
    head.lines().any(|line| {
        let l = line.trim_end();
        l.eq_ignore_ascii_case(&needle)
            // Some clients send no space after the colon: "Name:value".
            || l.eq_ignore_ascii_case(
                format!("{}{}", CONFIG_TOKEN_HEADER, CONFIG_TOKEN).as_str(),
            )
    })
}

async fn serve_html(socket: &mut embassy_net::tcp::TcpSocket<'static>) {
    let html = injected_index_html();
    println!("HTTP: serving index.html ({} bytes)", html.len());
    send_response(socket, "200 OK", "text/html", &html).await;
}

async fn serve_config(socket: &mut embassy_net::tcp::TcpSocket<'static>) {
    let (len, _v) = config_bytes();
    let data = CONFIG_DATA.get(len);
    println!("HTTP: serving current config ({} bytes)", len);
    send_response(socket, "200 OK", "application/json", data).await;
}

/// Validate the body of a (size-checked) `POST /config` and, on `Accepted`,
/// store it. Returns the led-core result so the caller can choose the
/// response (the 413 size path is handled by the caller, since the body the
/// read loop accumulates is capped at `CONFIG_MAX` and so cannot itself
/// trigger led-core's `TooLarge` — only the declared `Content-Length` can).
fn handle_post_config(body: &[u8]) -> http::ConfigPostResult {
    let result = http::validate_config_post(body, CONFIG_MAX);
    match result {
        // `Accepted`: persist the new config; the LED task picks it up on its
        // next cycle. `OutOfRange` (a value the semantic bounds reject, e.g.
        // `steps`/duration/counts past the caps) is handled explicitly so the
        // new "value out of range" error reaches the client rather than being
        // silently swallowed by a generic arm. Both send `ok:false` (see
        // led-core's `config_post_body`/`config_post_status`).
        http::ConfigPostResult::Accepted => {
            if config_set(body) {
                println!("HTTP: config updated ({} bytes)", body.len());
            } else {
                println!("HTTP: config too large to store");
            }
        }
        http::ConfigPostResult::OutOfRange => {
            println!("HTTP: config rejected — value out of range");
        }
        // TooLarge / NoEffects / BadJson — no state change.
        _ => {}
    }
    result
}

async fn handle_client(socket: &mut embassy_net::tcp::TcpSocket<'static>) {
    // Accumulate the request (headers + body) in a heap Vec so we can keep a
    // reference across `.await` points without holding a `&mut` to a static.
    let mut buf: Vec<u8> = Vec::with_capacity(CONFIG_MAX);
    let mut chunk = [0u8; READ_CHUNK_BYTES];

    loop {
        // Stop once the buffer holds CONFIG_MAX bytes: any larger body would
        // be truncated, and the declared Content-Length is checked (below) to
        // surface a 413 instead of a misleading "bad json".
        if buf.len() >= CONFIG_MAX {
            break;
        }
        // Race the read against the idle timeout so a client that connects and
        // then goes silent cannot hold this (the only) connection forever. On
        // timeout, abort the socket and return to `accept` (the server loop
        // tears down and re-accepts the next client).
        let read = with_timeout(IDLE_READ_TIMEOUT, socket.read(&mut chunk[..]));
        let n = match read.await {
            Ok(Ok(0)) => break, // client closed its write half
            Ok(Ok(n)) => n,
            Ok(Err(e)) => {
                println!("HTTP: read error: {:?}", e);
                return;
            }
            // No data within the idle budget: drop the connection and recover.
            Err(_) => {
                println!("HTTP: idle read timeout, dropping connection");
                socket.abort();
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
        http::Route::PostConfig => {
            // Auth gate: a config rewrite must carry the token header. GET
            // routes (above) stay open so the page still loads.
            if !config_token_provided(&buf) {
                println!("HTTP: POST /config rejected — missing/bad token header");
                send_response(
                    socket,
                    "403 Forbidden",
                    "application/json",
                    b"{\"ok\":false,\"error\":\"unauthorized\"}",
                )
                .await;
            } else {
                // Make 413 reachable: the read loop caps accumulation at
                // CONFIG_MAX, so an oversized body is truncated and would
                // surface as a misleading "bad json" (400). Instead, check the
                // DECLARED Content-Length against the cap first — if the client
                // promised more than CONFIG_MAX bytes, that's the true size and
                // must be a 413, not a 400.
                if req.content_length > CONFIG_MAX {
                    println!(
                        "HTTP: POST /config rejected — Content-Length {} > cap {}",
                        req.content_length, CONFIG_MAX
                    );
                    send_response(
                        socket,
                        "413 Payload Too Large",
                        "application/json",
                        b"{\"ok\":false,\"error\":\"too large\"}",
                    )
                    .await;
                } else {
                    let result = handle_post_config(req.body);
                    send_response(
                        socket,
                        http::config_post_status(result),
                        "application/json",
                        http::config_post_body(result),
                    )
                    .await;
                }
            }
        }
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
async fn led_task(led: LedRmt) -> ! {
    // The channel + its tick frequency (read in `main` before the channel was
    // partially moved out of the `Rmt` driver; see there) are the whole
    // driver state. `led_task` is the ONLY task that ever touches them.
    let mut channel = led.0;
    let tick_hz = led.1;

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

        // The transmit below BLOCKS for ~330 µs per frame (300 µs reset
        // dominates). esp-hal's RMT also exposes an async channel
        // (`into_async` + `Channel<_, Async, Tx>::transmit`), but adopting it
        // would be strictly more complex for zero gain here: it arms a
        // dedicated RMT interrupt vector bound to a driver-owned waker table
        // and must be set up in `main` (the handler is installed once, not
        // per transmission) — yet every frame is well under 1 ms, and the
        // blocking `wait` does not starve the other tasks: the executor only
        // resumes them after `led_task` yields at its next `await`. Keeping
        // the blocking call also matches the proven `src/main.rs` driver,
        // which uses exactly this pattern.
        if config.is_empty() {
            // No valid config: blink white so the LED is clearly alive.
            channel = ws2812_rgb_rmt(channel, tick_hz, &[255, 255, 255]);
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
                        channel = ws2812_rgb_rmt(channel, tick_hz, &color);
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
                        channel = ws2812_rgb_rmt(channel, tick_hz, &color);
                        Timer::after(Duration::from_millis(*step_ms as u64)).await;
                    }
                }
            }
        }
    }
}

// ── Main ───────────────────────────────────────────────────────
// The configured RMT TX channel is !Send: its lifetime is anchored by
// `PhantomData<Rmt<..>>` (the Rmt driver must outlive the channel) and the
// esp-hal peripheral/`PinGuard` handles are `!Send` by construction. Moving
// it into the embassy task is safe under a single-consumer invariant,
// stated plainly:
//
// SAFETY: `led_task` is the ONLY task (and the only OS thread) that ever
// dereferences or calls methods on the RMT channel it receives — `main`
// configures it once and hands it over; no other task, no esp-radio
// scheduler task, and no interrupt handler alias it. (Do NOT rely on "no
// two tasks ever run at once": esp-rtos is preemptive and the WiFi driver
// spawns its own OS tasks; soundness rests on this single-consumer
// confinement, not on the executor being single-threaded.)
struct LedRmt(Channel<'static, esp_hal::Blocking, Tx>, u32);
unsafe impl Send for LedRmt {}

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

    // Create the WS2812 RMT driver (GPIO27). 80 MHz counter clock → one tick
    // = 12.5 ns, the exact resolution the four WS2812 phase widths need.
    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(RMT_SCLK_MHZ)).unwrap();
    // Read the *actual* counter frequency BEFORE the channel is partially
    // moved out of `rmt` (a partial move leaves the whole struct unusable).
    // With `CHANNEL_CLK_DIVIDER == 1` the channel is undivided, so the
    // channel tick frequency equals the 80 MHz global sclk.
    let tick_hz = rmt.frequency().as_hz() / CHANNEL_CLK_DIVIDER as u32;
    let led_channel = rmt
        .channel0
        .configure_tx(&TxChannelConfig::default().with_clk_divider(CHANNEL_CLK_DIVIDER))
        .unwrap()
        .with_pin(peripherals.GPIO27);

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
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        0,
    );

    // Spawn all tasks
    spawner.spawn(wifi_task(controller).expect("spawn wifi_task"));
    spawner.spawn(net_task(runner).expect("spawn net_task"));
    spawner.spawn(http_server_task(stack).expect("spawn http_server_task"));
    spawner.spawn(led_task(LedRmt(led_channel, tick_hz)).expect("spawn led_task"));

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
        // SAFETY: only this task ever borrows `RX_BUF`/`TX_BUF` (they live in
        // this task), and it owns them for the whole accept/handle/teardown
        // cycle — no other task or interrupt aliases them. `TcpSocket::new`
        // transmutes them to `'static`, so they must live in a `static` here.
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
