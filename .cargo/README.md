# .cargo

Cargo configuration for the repo. One file, `config.toml`. It is the source of
truth for the firmware build target and the flash runner, and it is what makes
a plain host-side `cargo test` fail.

## What `config.toml` sets

- **`[build] target = "riscv32imac-unknown-none-elf"`** — every `cargo`
  invocation in this repo (build, run, test) defaults to the RISC-V firmware
  target. That is why `cargo build` / `cargo run` produce the firmware with no
  `--target` flag.
- **`[target.riscv32imac-unknown-none-elf] runner`** =
  `espflash flash --monitor --chip esp32c5` — the runner `cargo run` uses to
  flash the built binary and open the serial monitor.
- **`rustflags = ["-C", "link-arg=-Tlinkall.x"]`** — a linker flag forcing
  inclusion of all archive members. The `thirdparty/` README notes this is the
  only link wiring this file contributes (no search paths).
- **`[env] MONITOR_BAUD = "115200"`** — the serial-monitor baud rate.
- **`[unstable] build-std = ["alloc", "core"]`** — the firmware target has no
  prebuilt std, so `alloc` and `core` are built from source (the `rust-src`
  component, pinned by `rust-toolchain.toml`). The `std` and `test` crates are
  **not** in this list.

## Consequence: a plain `cargo test` fails here

Because this file forces the RISC-V target and a `build-std` list that omits
`test`, any host-side `cargo test` (at the root or in `led-core/`) fails before
the test harness runs, with "can't find crate for `test`". The repo documents
this in three places — keep them consistent:

- `README.md:143` ("Plain `cargo test` fails hard" — in both packages),
- `scripts/README.md:5`–`9`, and
- `led-core/README.md:52`.

The working host-test invocation overrides **both** the target and the
`build-std` list, e.g.
`cargo test --manifest-path led-core/Cargo.toml --target
x86_64-unknown-linux-gnu --config 'unstable.build-std=["std"]'` (see
`scripts/README.md` and `led-core/tests/README.md`). Note the root `README.md`
and `led-core/README.md` use `build-std=["std","test"]` instead of `["std"]`;
both work because either list supplies the `test` crate.
