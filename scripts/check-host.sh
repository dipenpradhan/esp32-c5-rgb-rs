#!/usr/bin/env bash
# Host-side clippy + fmt checks for the pure `led-core` crate.
#
# The repo root .cargo/config.toml sets the RISC-V firmware target and
# -Zbuild-std (the target has no prebuilt std, and its build-std list omits
# the `test` crate, so host test targets cannot build for it). For host
# checks we build for x86_64; because the root config forces build-std, we
# build the full host std from source (["std"]) so the from-source core/alloc
# and prebuilt std agree.
#
# Scope: led-core only. The root `esp-c5-hello` package depends on esp-hal,
# whose build script refuses to build for any target other than the chip's,
# so the firmware crate itself cannot be checked on the host.
set -euo pipefail
cd "$(dirname "$0")/.."
echo "── clippy (host) ──"
cargo clippy --all-targets \
  --manifest-path led-core/Cargo.toml \
  --target x86_64-unknown-linux-gnu \
  --config 'unstable.build-std=["std"]'
# `led-core` is a separate package (no [workspace] table, own Cargo.lock), so a
# root-level `cargo fmt` does NOT reach it - each package is checked explicitly.
echo "── fmt (check: firmware) ──"
cargo fmt --check
echo "── fmt (check: led-core) ──"
cargo fmt --check --manifest-path led-core/Cargo.toml
