# led-core/tests

Integration tests for the `led-core` crate. Each file is its own test binary
(auto-discovered from this directory) and asserts on the *observable* behavior
of one subsystem by driving the real `led-core` code with synthetic inputs —
not on the shipped config files, so a legitimate edit to
`configs/effects.json` cannot break them.

## The files

- **`led_effects.rs`** — the overall LED behavior. Builds a synthetic
  multi-effect config (defined inline in the file), runs `cycle_steps`, and
  asserts the exact frame sequence and timing for one full cycle, that a
  `POST /config`-shaped JSON changes the next cycle, and that an
  empty/invalid config leaves the LED alive rather than dead. Frames are
  produced through the **legacy** `encode_rgb` (`tests/led_effects.rs:19`) —
  this test verifies the expansion logic, and the firmware's actual waveform
  comes from the `V2` encoder (see `src/README.md`).
- **`web_server.rs`** — the on-device web server. Feeds the real `http`
  pipeline raw HTTP byte streams, including requests split across multiple
  reads (partial headers and bodies), and walks a full client session:
  `GET /`, `GET /config`, `POST /config` (valid → stored), `GET /config`
  (returns the new config), and `POST /config` with invalid / too-large bodies
  (400 / 413). The "store" is a `Vec<u8>` mirroring the firmware's config
  buffer.
- **`wifi_connection.rs`** — the host-side WiFi setup. Verifies the pipeline the
  firmware runs *before* touching the (target-only) radio driver: the
  credentials parse, validate against the ESP32 constraints, are the exact
  values handed to the driver, and invalid credentials are rejected before the
  driver is invoked.

## Running them

These do **not** run under a plain `cargo test`. The root `.cargo/config.toml`
forces the RISC-V firmware target and `-Zbuild-std = ["alloc", "core"]` — the
`test` crate is not in that list, so test targets cannot build
("can't find crate for `test`"). The working invocation is the one in
`scripts/test-led-core.sh`:

```bash
cargo test --manifest-path led-core/Cargo.toml --target x86_64-unknown-linux-gnu --config 'unstable.build-std=["std"]'
```

(or `bash scripts/test-led-core.sh`). Run it after touching anything in
`led-core/` or anything in `configs/`, since the crate's tests read those
files.
