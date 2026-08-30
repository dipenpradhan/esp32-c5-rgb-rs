# scripts

Host-side test and lint gates for the `led-core` crate. They exist because two
things about this repo are non-obvious:

1. The root `.cargo/config.toml` forces the RISC-V firmware target and
   `-Zbuild-std = ["alloc", "core"]` — the `test` crate is not in that list, so
   any host-side cargo invocation must explicitly override both the target and
   the build-std list, or it fails with "can't find crate for `test`".
2. The root is not a cargo workspace (no `[workspace]` table), so a root
   `cargo fmt` / `cargo clippy` does not reach the separate `led-core` package
   at all.

Also note: the firmware package itself cannot be checked on the host —
esp-hal's build script refuses to build for any target other than the chip's.
These scripts cover `led-core` only.

## test-led-core.sh

Runs the `led-core` unit + integration tests on the host (85 tests):

```bash
cargo test --manifest-path led-core/Cargo.toml --target x86_64-unknown-linux-gnu --config 'unstable.build-std=["std"]'
```

Use it after touching anything in `led-core/` — or anything in `configs/`,
since `led-core`'s tests read those files.

## check-host.sh

Host lint/format gate — run before pushing:

- `cargo clippy --all-targets --manifest-path led-core/Cargo.toml --target x86_64-unknown-linux-gnu --config 'unstable.build-std=["std"]'`
- `cargo fmt --check` (root firmware package)
- `cargo fmt --check --manifest-path led-core/Cargo.toml`

Fails on any clippy warning (lints are denied by default in both packages) or
any formatting difference.
