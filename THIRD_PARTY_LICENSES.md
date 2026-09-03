# Third-party licenses

This repository vendors a small number of third-party components under
`thirdparty/`. Each is listed below with its location, origin, and the
licence that governs it.

| Component | Location | Origin | License |
|---|---|---|---|
| `esp-wifi-sys-esp32c5` crate source (Rust bindings, `build.rs`, `ftm_stubs.c`) | `thirdparty/esp-wifi-sys-esp32c5/{src,build.rs,ftm_stubs.c}` | [esp-rs/esp-wifi-sys](https://github.com/esp-rs/esp-wifi-sys), declared in its `Cargo.toml` | MIT OR Apache-2.0, declared in its `Cargo.toml` |
| Espressif prebuilt WiFi/PHY binaries (15 `.a` archives) | `thirdparty/esp-wifi-sys-esp32c5/libs/` | Not yet established | Not yet established |

The licence for the prebuilt binaries under `thirdparty/esp-wifi-sys-esp32c5/libs/`
has not yet been established: no `LICENSE`, `COPYING`, or `NOTICE` file ships
with them, and their provenance has not been recorded. This is tracked as an
open item, and the licence will be recorded here once confirmed.

Only the 15 `.a` archives are tracked in git and redistributed. Intermediate
`.o` object files may exist in a local working tree; they are gitignored, are
not referenced by `build.rs`, and are not distributed.

All other dependencies (crates.io and esp-rs crates) are fetched from their
origins at build time under their own licences and are not redistributed by
this repository. `forks/` is gitignored and is not distributed with the
repository.
