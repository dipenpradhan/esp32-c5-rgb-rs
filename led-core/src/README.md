# led-core/src

The source of the `led-core` crate: the pure, hardware-independent logic.
`lib.rs` is `#![no_std]` with `extern crate alloc` (`src/lib.rs:24`); the only
dependencies are `serde` and `serde_json`, both `default-features = false`
with the `alloc` feature (`Cargo.toml`), so the crate compiles and its tests
run on the host with no hardware. Every rustc/clippy warning is a hard error
(`[lints.rust] warnings = "deny"`, `[lints.clippy] all = "deny"` in
`Cargo.toml`). `lib.rs` re-exports the most-used items
(`LedConfig`, `LedEffect`, the `ws2812` encoders).

## Modules

- **`config`** — the effects config model and JSON parsing. `LedConfig` (a
  list of effects) and the tagged `LedEffect` enum with the two effect kinds,
  `blink` and `blend` (`src/config.rs:18`, `:39`). `parse_config` /
  `parse_config_bytes` parse JSON into the model (`src/config.rs:79`, `:84`),
  returning `Err` on malformed JSON or a missing/unknown effect. A test embeds
  the shipped `configs/effects.json` with `include_str!`
  (`src/config.rs:93`) but asserts only that it parses and is non-empty.
- **`effects`** — expands a config into the flat list of `(color, hold)` steps
  one full LED cycle emits. `cycle_steps` produces the exact frames in order
  (`src/effects.rs:31`); `cycle_duration_ms` and `cycle_frame_count` sum the
  per-effect totals (`src/effects.rs:65`, `:70`). A `blend` expands to
  `steps + 1` frames over `0..=steps`, so its endpoints land exactly on
  `from` and `to`.
- **`color`** — RGB math for the `blend` effect. Colors are `[R, G, B]`
  (`Rgb`, `src/color.rs:5`); `interpolate` / `lerp_u8` do the per-channel
  lerp in `i64` so a `u32` step cannot overflow `i32` (`src/color.rs:10`,
  `:30`). `total == 0` returns `from` rather than dividing.
- **`ws2812`** — WS2812 wire-protocol encoding. See below; this is the module
  with a load-bearing subtlety.
- **`http`** — the pure half of the on-device web server: HTTP/1.1 request
  parsing, routing, and response-header building over byte slices only, with no
  I/O. `parse_request` / `route` / `validate_config_post` /
  `build_response_header` (`src/http.rs:130`, `:151`, `:234`, `:280`). It
  validates the *body* of a `POST /config`; the per-config token header that
  gates that route is enforced in the firmware (`examples/web_server.rs`), not
  here. `RESPONSE_HEADER_BYTES` sizes the header buffer
  (`src/http.rs:278`).
- **`wifi`** — the WiFi-credentials model. `WifiCreds` and `validate` check the
  ESP32 constraints (SSID 1–32 bytes, WPA2-PSK password 8–63 chars or empty for
  an open network) (`src/wifi.rs:15`, `:24`). `WIFI_CONFIG_JSON` embeds
  `configs/wifi.json` via `include_str!` (`src/wifi.rs:47`), so a build fails
  if that untracked file is missing.

## `ws2812` — two encoders, only one ships

`src/ws2812.rs` contains **both** a legacy model (the first half of the file)
and a protocol-compliant `V2` model (after the marker near
`src/ws2812.rs:377`). Both are public and re-exported, so a reader cannot tell
from the API which one the firmware uses:

- **Legacy** (`Ws2812Event`, `encode_rgb` / `encode_grb`, `replay_frame`, the
  `PinPulse` trait; `src/ws2812.rs:54`, `:152`, `:187`): one 1 µs phase per
  bit, µs granularity. This is **non-compliant** — it cannot express the
  datasheet's two timed phases, so consecutive `1` bits merge and the 24-bit
  GRB frame misaligns. It produces garbled color on real hardware. It is kept
  only for the host tests that exercise it and for the integration test
  `tests/led_effects.rs`, which builds frames through it.
- **V2** (`Ws2812BitPhase`, `encode_rgb_v2` / `encode_grb_v2`,
  `replay_frame_v2`, the `PinPulseNs` trait; `src/ws2812.rs:463`, `:530`,
  `:573`): both phases of every bit carried explicitly in nanoseconds
  (T0H 400 / T0L 850, T1H 800 / T1L 450 ns; `src/ws2812.rs:387` onward), 24
  bits in GRB order, and a 300 µs reset (`RESET_NS`, ≥ the WS2812B 280 µs
  spec).

**Only the V2 path is what the firmware ships.** `src/main.rs` and the
LED-driving examples all call `encode_rgb_v2` and build their 25-entry RMT
pulse-code frame from the `V2` frame (`src/main.rs:34`, `:86`, `:111`); none
call `replay_frame`, `encode_rgb` (non-v2), or anything `GpioPulse`-based. The
shipped firmware does not bit-bang at all — it drives the LEDs through the
RMT peripheral. `replay_frame` / `replay_frame_v2` and the `PinPulse` /
`PinPulseNs` traits are host test harnesses only; the tests verify the encoded
waveform (both phases present, in-window, a measurable LOW between `1` bits,
a spec-length reset), not the on-device hardware timing.
