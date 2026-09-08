# thirdparty

A single vendored crate — `esp-wifi-sys-esp32c5/` — that carries the prebuilt
Espressif WiFi/PHY binary blobs the ESP32-C5 firmware links against. Nothing
else under `thirdparty/` is a source dependency.

## What is in here

`esp-wifi-sys-esp32c5/` is a local copy of the `esp-wifi-sys-esp32c5` crate
(Rust bindings to Espressif's WiFi/Bluetooth low-level drivers for the chip):

- `src/` — the Rust bindings (`lib.rs`, `c_types.rs`, `fmt.rs`, `include.rs`).
- `build.rs` — the link wiring; see below.
- `Cargo.toml` — declares `license = "MIT OR Apache-2.0"` (that licence covers
  the **Rust bindings**, not the binary blobs).
- `libs/` — the binary blobs:
  - **14 Espressif prebuilt `.a` archives** (WiFi/PHY and the Bluetooth
    libraries it needs): `libble_app`, `libbtbb`, `libcoexist`, `libcore`,
    `libespnow`, `libmesh`, `libnet80211`, `libphy`, `libpp`, `libprintf`,
    `libregulatory`, `libsmartconfig`, `libwapi`, `libwpa_supplicant`.
  - A set of `.o` object files (`ieee80211_*.o`, `wl_*.o`, `test*.o`,
    `if_eagle.o`) that are **intermediates**: they are
    gitignored (`*.o` in the crate's own `.gitignore`), not tracked, and not
    referenced by `build.rs`. They may exist in a local working tree but are
    not distributed.

So the count: **14 `.a` archives**, all Espressif prebuilt binaries. All 14
`.a` archives (and the crate source) are the only things tracked in git here.

## Provenance

Recorded and established in `THIRD_PARTY_LICENSES.md`. Precisely:

- All **14** archives in `libs/` are a byte-for-byte snapshot of
  [esp-rs/esp-wifi-sys](https://github.com/esp-rs/esp-wifi-sys) at commit
  `fdf0095b1c` — the state immediately after the upstream "Update v5.5.3 (#498)"
  commit — and correspond to **ESP-IDF v5.5.3**.

## Licence

Apache-2.0, Copyright Espressif Systems — the same licence as ESP-IDF, per
Espressif's published statement covering its prebuilt WiFi/PHY binary libraries
(esp-phy-lib, esp32-wifi-lib). Plainly: **no `LICENSE`, `COPYING` or `NOTICE`
file is bundled inside the archives themselves**; the Apache-2.0 attribution is
recorded from Espressif's statement, not from any file present in this repo. The
crate's `MIT OR Apache-2.0` tag applies to the Rust bindings only.

## Why these blobs are needed and how the build consumes them

Espressif ships the WiFi/PHY driver as closed binary archives rather than
source, so the firmware must link prebuilt `.a` files. The consumption path:

1. **`[patch.crates-io]` in the root `Cargo.toml`** redirects the
   `esp-wifi-sys-esp32c5` dependency to this local path
   (`{ path = "thirdparty/esp-wifi-sys-esp32c5" }`). That is what makes cargo
   use these blobs instead of whatever crates.io would provide.
2. The crate is pulled in **transitively and only when WiFi is enabled**:
   `esp-radio` (an optional dependency, behind the `wifi` feature, which
   `wifi-net` and `webserver` also pull in) depends on the per-chip
   `esp-wifi-sys-esp32c5`. The default binary and the non-WiFi examples do not
   enable `wifi`, so the default `cargo build` does **not** build or link this
   crate. Building with `--features wifi` / `wifi-net` / `webserver` (e.g. the
   WiFi and web-server examples) does.
3. **`build.rs`** does the actual linking. For each of the 14 Espressif
   archives it copies `libs/lib<name>.a` into the crate's `OUT_DIR` and emits
   `cargo:rustc-link-lib=<name>`; then it emits a single
   `cargo:rustc-link-search=<OUT_DIR>`. That emitted search path plus the
   per-library link directives are the entire link/search-path wiring. The root
   `.cargo/config.toml` contributes only `-C link-arg=-Tlinkall.x` (a linker
   script flag that forces inclusion of all archive members); it adds no
   search paths.
