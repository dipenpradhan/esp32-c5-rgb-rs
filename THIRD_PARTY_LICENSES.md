# Third-party licenses

This repository vendors a small number of third-party components under
`thirdparty/`. Each is listed below with its location, origin, and the
licence that governs it.

| Component | Location | Origin | License |
|---|---|---|---|
| `esp-wifi-sys-esp32c5` crate source (Rust bindings, `build.rs`, `ftm_stubs.c`) | `thirdparty/esp-wifi-sys-esp32c5/{src,build.rs,ftm_stubs.c}` | [esp-rs/esp-wifi-sys](https://github.com/esp-rs/esp-wifi-sys), declared in its `Cargo.toml` | MIT OR Apache-2.0, declared in its `Cargo.toml` |
| Espressif prebuilt WiFi/PHY binaries (15 `.a` archives) | `thirdparty/esp-wifi-sys-esp32c5/libs/` | 14 of the 15 are ESP-IDF v5.5.3 blobs, a byte-for-byte snapshot of [esp-rs/esp-wifi-sys](https://github.com/esp-rs/esp-wifi-sys) at commit `fdf0095b1c` (the state immediately after the upstream "Update v5.5.3 (#498)" commit); `libftm_stubs.a` is built locally from `ftm_stubs.c` | Apache-2.0 (the same licence as ESP-IDF), Copyright Espressif Systems, per Espressif's statement for its prebuilt WiFi/PHY binary libraries; not bundled in-file |

The 14 Espressif prebuilt WiFi/PHY binaries under `thirdparty/esp-wifi-sys-esp32c5/libs/`
are a byte-for-byte snapshot of [esp-rs/esp-wifi-sys](https://github.com/esp-rs/esp-wifi-sys)
at commit `fdf0095b1c`, the state immediately after the upstream "Update v5.5.3 (#498)"
commit; they correspond to ESP-IDF v5.5.3. `libftm_stubs.a` in the same directory is a
local stub compiled from `ftm_stubs.c` and is not an Espressif binary. The prebuilt
binaries are provided by Espressif under the same licence as ESP-IDF — Apache-2.0,
Copyright Espressif Systems, in binary ("Object") form — per Espressif's published
statement for its prebuilt WiFi/PHY libraries (esp-phy-lib, esp32-wifi-lib). No
`LICENSE`, `COPYING`, or `NOTICE` file is bundled with the shipped archives; the Apache-2.0
attribution is recorded from Espressif's statement rather than from a file present in
this repository. The MIT OR Apache-2.0 declared in the crate's `Cargo.toml` applies
to the Rust bindings, not to these binaries.

Only the 15 `.a` archives are tracked in git and redistributed. Intermediate
`.o` object files may exist in a local working tree; they are gitignored, are
not referenced by `build.rs`, and are not distributed.

All other dependencies (crates.io and esp-rs crates) are fetched from their
origins at build time under their own licences and are not redistributed by
this repository. `forks/` is gitignored and is not distributed with the
repository.
