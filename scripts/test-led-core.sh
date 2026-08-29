#!/usr/bin/env bash
# Host-side unit + integration tests for the pure `led-core` crate.
#
# The repo root .cargo/config.toml sets the RISC-V firmware target and
# -Zbuild-std (the target has no prebuilt std). For host tests we build for
# x86_64; because the root config forces build-std, we build the full host std
# from source (["std"]) so the from-source core/alloc and prebuilt std agree.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test \
  --manifest-path led-core/Cargo.toml \
  --target x86_64-unknown-linux-gnu \
  --config 'unstable.build-std=["std"]'
