# led-core

The pure, hardware-independent logic crate for this project. Everything that
can be written without a chip lives here so it can be unit- and
integration-tested on the host; the firmware (root package + `examples/`) is a
thin hardware layer that consumes this crate.

## Purity contract

- No I/O, no hardware dependencies. The only dependencies are `serde` and
  `serde_json` (both `default-features = false`, `alloc` feature).
- `#![no_std]` with `extern crate alloc`; compiles for the RISC-V firmware
  target and for the host alike.
- Strict lints: every rustc/clippy warning is a hard error.

Why it exists: the effects, WS2812 timing, HTTP parsing and credential
validation are the fiddly parts of this project, and all of it is verifiable
without hardware. 85 host tests currently pass.

## The two seams

- **`ws2812::PinPulse` trait.** WS2812 encoding is modelled as a stream of
  timed `Ws2812Event`s; `replay_frame` drives whatever `PinPulse`
  implementation it is given. The firmware supplies real GPIO; tests supply a
  recorder. The tested GPIO sequence is therefore exactly the shipped one.
- **I/O-free `http` module.** Request parsing, routing and response building
  take byte slices only — no sockets. Tests replay split TCP reads through the
  real parser.

## Modules

- `config` — the effects config model (`configs/effects.json`) and JSON
  parsing/validation.
- `effects` — expands a config into the flat sequence of (colour, hold) steps
  the LED emits for one full cycle.
- `color` — RGB colour interpolation (blend-effect math).
- `ws2812` — WS2812 wire-protocol encoding (bit order, GRB order, reset pulse)
  plus `replay_frame` and the `PinPulse` trait.
- `http` — minimal HTTP request parsing, routing and response building for
  the on-device web server.
- `wifi` — WiFi credentials model (single source of truth: `configs/wifi.json`)
  and ESP32 constraint validation.

## Running the tests

```bash
cargo test -p led-core --target x86_64-unknown-linux-gnu --config 'unstable.build-std=["std","test"]'
```

(or `bash scripts/test-led-core.sh`). 85 tests.

Plain `cargo test` fails in this directory too: the root
`.cargo/config.toml` forces the RISC-V firmware target and a `-Zbuild-std`
list of `["alloc", "core"]` — the `test` crate is not in that list, so test
targets cannot build ("can't find crate for `test`"). The explicit invocation
above overrides both the target and the build-std list, which is why it is
mandatory.

## Caveat: not yet fully self-contained

Despite the purity contract, this crate currently reaches outside its own
directory in three places:

- `src/config.rs:93` and `src/effects.rs:166`: `include_str!("../../configs/effects.json")`
- `src/wifi.rs:47`: `include_str!("../../configs/wifi.json")`

That couples this crate's tests to files owned by the other package (and means
several tests anchor to the exact contents of `configs/effects.json`). This is
known and slated to change.
