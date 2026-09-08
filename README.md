# ESP32-C5 RGB LED (esp32-c5-rgb-rs)

Firmware for the ESP32-C5-DevKitC-1 that drives the on-board WS2812 RGB LED
(GPIO27, GRB order) with JSON-configured effects and runs a WiFi HTTP server
on port 80 serving a single-page config UI with live two-way config updates.

## What the hardware does

- Runs a list of LED effects from `configs/effects.json` in a loop. Two effect
  shapes exist: `blink` (cycle through a list of colours) and `blend`
  (interpolate between two colours over N steps).
- Serves a web UI over WiFi once it has an IP:
  - `GET /` — the embedded config UI page
  - `GET /config` — current effect config JSON
  - `POST /config` — replace the effect config (validated); the LED task picks
    it up live on its next cycle.

Config set via `POST /config` lives in RAM only and is lost on reboot — see
Known issues.

## Architecture

This is NOT a cargo workspace — two independent packages, each with its own
`Cargo.lock`, plus a UI directory:

| Directory | What it is |
|---|---|
| `.` (root, `esp32-c5-rgb-rs`) | The firmware. `no_std`, target `riscv32imac-unknown-none-elf`. `src/main.rs` (the default binary) plus 9 binaries in `examples/`. |
| `led-core/` | Pure `no_std` logic: config model, effect expansion, colour math, WS2812 protocol, HTTP parsing, WiFi credential validation. Only dependencies are `serde` + `serde_json`. Host-tested: 107 tests, all passing. |
| `web/` | TypeScript + Vite UI. Builds to a single self-contained `web/dist/index.html` (16292 bytes), which the firmware embeds via `include_str!`. |

The design principle: all logic lives in host-testable `led-core`; the
firmware is a thin hardware layer that replays what `led-core` computed. Two
seams make that possible:

- the `PinPulse` trait (`led_core::ws2812`) — the firmware supplies real GPIO,
  tests supply a recorder, so the tested GPIO sequence is exactly the shipped
  one; and
- the I/O-free `http` module — tests replay split TCP reads through the real
  parser.

`web/dist/index.html` is deliberately tracked in git — it is a firmware build
input, not an ordinary build artifact.

## Prerequisites

- Rust nightly — `rust-toolchain.toml` pins `channel = "nightly"` and
  `components = ["rust-src"]` (rust-src is needed for `-Zbuild-std`).
- Target `riscv32imac-unknown-none-elf`, built from source via
  `-Zbuild-std` (`[unstable] build-std = ["alloc", "core"]` in
  `.cargo/config.toml`).
- `espflash`, for flashing. No version is pinned by the repository
  (`cargo install espflash`); the development environment has 4.5.0.
- Node.js + npm, only if you will change the web UI.
- Board: ESP32-C5 rev v1.0, 4 MB flash, on `/dev/ttyACM0` (the board's native
  USB-JTAG port; `/dev/ttyUSB0` is a CH340 bridge to the same chip).

## Commands

### Quick start

```bash
./build.sh
```

Builds the firmware from a fresh clone. It checks the toolchain, then builds;
it prints the artifact path when done. If `configs/wifi.json` is missing it
seeds the file from `configs/wifi.json.example`, which contains placeholder
values — put your real WiFi credentials in `configs/wifi.json` before flashing
anything that uses WiFi. Flashing is a separate manual step (below) and needs
`espflash`; `build.sh` warns if it is absent but does not install it. The
manual steps that follow are what `build.sh` wraps, for when you want to run
them individually.

### Host tests for led-core

```bash
cargo test -p led-core --target x86_64-unknown-linux-gnu --config 'unstable.build-std=["std","test"]'
```

Runs all 107 unit + integration tests on the host. The plain `cargo test` does
NOT work — see Known issues for why, and what to type instead.

### Host lint/format gate

```bash
bash scripts/check-host.sh
```

Clippy + `cargo fmt --check` for both packages.

### Firmware build

```bash
cargo check
cargo build
cargo check --examples --all-features
```

The target is inherited from `.cargo/config.toml`. Examples are feature-gated
— e.g. `examples/web_server.rs` needs `--features webserver` — and
`cargo check --examples --all-features` compiles all 9 examples clean.

### Flash to the board

```bash
cargo run --example led_effects_sync
```

uses the configured runner (`espflash flash --monitor --chip esp32c5`), or
explicitly:

```bash
cargo espflash flash --example <name> --chip esp32c5 --port /dev/ttyACM0 --monitor
```

### Rebuild the web UI

```bash
cd web
npm install
npm run build
```

Regenerates `web/dist/index.html`. A UI change does nothing until this is run
AND the firmware is rebuilt and flashed. See `web/README.md` for the size
budget.

## configs/

- `effects.json` — the LED effect list. Embedded at compile time via
  `include_str!` and also the runtime default. Two shapes: `blink`
  (`colors` + `duration_ms`) and `blend` (`from`/`to`/`steps`/`step_ms`).
- `wifi.json` — WiFi credentials, embedded into the firmware binary at compile
  time. It must contain real credentials before any WiFi example works. It is
  untracked and gitignored (copy `wifi.json.example` and fill in real values) —
  but because it is compiled in, see Known issues, caveat 2.

## Known issues / caveats

1. **Plain `cargo test` fails hard** ("can't find crate for `test`") in BOTH the
   repo root and inside `led-core/`. The root
   `.cargo/config.toml` forces the RISC-V firmware target and a
   `-Zbuild-std` list of `["alloc", "core"]` — the `test` crate is not in that
   list, so test targets cannot be built for that target. At the root it is a
   single error; inside `led-core/` (a `no_std` crate) the missing `test` and
   `std` crates cascade into dozens more. You must override both the target
   and the build-std list explicitly, as in the host-test command above
   (or use `scripts/test-led-core.sh`).
2. **WiFi credentials are compiled into the firmware image.** `configs/wifi.json`
   is untracked and gitignored — copy `configs/wifi.json.example` (placeholder
   values only) to `configs/wifi.json` and fill in real credentials. The real
   file's contents are not in any committed tree, so a fresh clone will not
   contain them. But the file is embedded into the firmware binary via
   `include_str!`, so anyone who can dump the board's flash can recover the
   credentials. Treat any board you hand to someone else as disclosing that
   WiFi password.
3. **Runtime config is RAM-only.** Config set via `POST /config` is LOST ON
   REBOOT — the device reverts to the compile-time `configs/effects.json`.
   Documented as a known limitation, not a bug to chase.
4. **Prebuilt binary blobs.** `thirdparty/esp-wifi-sys-esp32c5` contains
   ~17 MB of prebuilt Espressif binary blobs; the build depends on them via a
   `[patch.crates-io]` entry. Origin and licence are recorded in
   [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
5. **esp-hal is pinned by git revision.** esp-hal and its companion crates
   resolve from a pinned upstream revision (see `Cargo.toml`), not a branch
   head, and are additionally pinned by `Cargo.lock`.

## Repository layout

```
.
├── Cargo.toml          # esp32-c5-rgb-rs firmware package; [patch.*] entries
├── .cargo/config.toml  # RISC-V target, espflash runner, -Zbuild-std
├── rust-toolchain.toml # nightly + rust-src
├── src/main.rs         # default binary: sync config-driven LED effects
├── examples/           # 9 further firmware binaries — see examples/README.md
├── led-core/           # pure no_std logic crate — see led-core/README.md
├── web/                # TS + Vite UI -> dist/index.html — see web/README.md
├── configs/            # effects.json, wifi.json — see configs/README.md
├── scripts/            # host test/lint scripts — see scripts/README.md
└── thirdparty/         # esp-wifi-sys-esp32c5 prebuilt blobs — see caveat 4
```

`forks/` is also present in a working tree but gitignored: local scratch
clones of upstream crates, not part of the buildable project.

## License

Copyright 2026 Dipen Pradhan

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

SPDX-License-Identifier: `Apache-2.0`. The full text is in [`LICENSE`](LICENSE).

### Third-party licenses

This repository vendors third-party components under `thirdparty/`,
which are governed by their own licences. The complete inventory,
including each component's origin and licence, is in
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
