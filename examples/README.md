# examples

Nine further firmware binaries, all `no_std` for
`riscv32imac-unknown-none-elf`. The default binary (`src/main.rs`) is the sync
config-driven effects runner; these are additional entry points: product
demos, minimal sanity examples, and hardware diagnostics. Every example uses
`led-core` for its logic — each file is a thin hardware layer.

## The examples

| Name | Purpose | Required features |
|---|---|---|
| `blocking_blink` | Minimal blocking GPIO blink — the smallest possible firmware | (none) |
| `async_blink` | Minimal async GPIO blink (embassy executor + systimer) | `async` |
| `led_effects_sync` | Product demo: config-driven effects, sync/blocking | (none) |
| `led_effects_async` | Product demo: config-driven effects, async/await | `async` |
| `web_server` | Product: WiFi HTTP server on port 80; serves the embedded UI with `GET /config` / `POST /config` live two-way updates. **This is currently the flagship entry point, despite living in `examples/`.** | `webserver` |
| `serial_test` | Diagnostics: verify `esp-println` serial output works | (none) |
| `wifi_dhcp` | Diagnostics: embassy-net DHCP/network stack (adapted from esp-hal) | `wifi-net` |
| `wifi_led_test` | Diagnostics: blocking WiFi init; each init phase writes a real WS2812 colour so the LED shows how far it got (on failure it blinks the numeric error code) | `wifi` |
| `wifi_test` | Diagnostics: WiFi bring-up with LED indication | `wifi-net` |

The features are declared as `required-features` on the `[[example]]` blocks
in the root `Cargo.toml`, so a plain `cargo build --examples` only compiles
the ones whose dependencies are enabled; `cargo check --examples
--all-features` compiles all 9.

## Flashing

```bash
cargo run --example <name>
```

uses the runner configured in `.cargo/config.toml`
(`espflash flash --monitor --chip esp32c5`), or explicitly:

```bash
cargo espflash flash --example <name> --chip esp32c5 --port /dev/ttyACM0 --monitor
```

Notes: WiFi examples read credentials from `configs/wifi.json` (see
`configs/README.md` — it must contain real credentials first), and
`web_server` serves `web/dist/index.html`, which must be rebuilt before a UI
change reaches the device (see `web/README.md`).
