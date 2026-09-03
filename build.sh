#!/usr/bin/env bash
#
# One-command build entrypoint for the esp32-c5-rgb-rs ESP32-C5 firmware.
#
# What it does, in order:
#   1. Checks the toolchain prerequisites (cargo, a nightly toolchain, the
#      rust-src component; espflash is only needed to flash, so it warns).
#   2. Seeds configs/wifi.json from the tracked example ONLY if it is absent
#      (never overwriting an existing file — it may hold real credentials).
#   3. Builds the firmware for the target configured in .cargo/config.toml.
#   4. Prints the artifact path and the next steps (host tests, flashing).
#
# It is safe to run repeatedly: it never modifies existing files and it never
# touches configs/wifi.json once that file exists.
set -euo pipefail
cd "$(dirname "$0")"

# ── helpers ──────────────────────────────────────────────────────────────────
info() { printf '\n[info] %s\n' "$*"; }
warn() { printf '\n[warn] %s\n' "$*"; }
die()  { printf '\n[error] %s\n' "$*" >&2; exit 1; }

# Horizontal-rule separators. Printed with `printf '%s\n' "$RULE"` (the rule is
# an ARGUMENT, never the format string) because a format string that begins
# with '-' is parsed as a printf option (a run of '--' → "invalid option") and
# aborts under `set -e`.
RULE_EQ='============================================================'
RULE_DASH='------------------------------------------------------------'
hr()   { printf '\n%s\n' "$RULE_EQ"; }
hr2()  { printf '\n%s\n' "$RULE_DASH"; }

# ── banner ───────────────────────────────────────────────────────────────────
hr
printf ' esp32-c5-rgb-rs — one-command firmware build\n'
printf ' Checks toolchain, seeds configs/wifi.json if missing,\n'
printf ' and builds the ESP32-C5 firmware.\n'
printf '%s\n' "$RULE_EQ"

# ── 1. prerequisite checks ───────────────────────────────────────────────────

# (a) cargo present
if command -v cargo >/dev/null 2>&1; then
  info "cargo found: $(cargo --version 2>/dev/null || echo '(version lookup failed)')"
else
  die "cargo was not found on PATH.
  Fix: install the Rust toolchain with rustup —
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  then open a NEW shell (so cargo is on PATH) and re-run this script."
fi

HAVE_RUSTUP=0
command -v rustup >/dev/null 2>&1 && HAVE_RUSTUP=1

# The project requires a *nightly* toolchain: .cargo/config.toml sets
# [unstable] build-std = ["alloc", "core"], and -Zbuild-std is nightly-only.
if [ "$HAVE_RUSTUP" -eq 1 ]; then
  # Which toolchain will cargo actually use here? (Honours rust-toolchain.toml.)
  ACTIVE_TOOLCHAIN=$(rustup show active-toolchain 2>/dev/null | head -n1 | awk '{print $1}') || ACTIVE_TOOLCHAIN=""
  if [ -z "$ACTIVE_TOOLCHAIN" ]; then
    die "rustup is installed but could not resolve an active toolchain.
  Fix: rustup default nightly   (or make sure a toolchain is installed: rustup toolchain list)"
  fi
  case "$ACTIVE_TOOLCHAIN" in
    nightly*) : ;;
    *) die "cargo will use toolchain '$ACTIVE_TOOLCHAIN', but this project needs a nightly toolchain (build-std is nightly-only; rust-toolchain.toml pins channel = \"nightly\").
  Fix: rustup toolchain install nightly && rustup default nightly"
       ;;
  esac
  # It must actually be installed, not merely pinned.
  if rustup toolchain list 2>/dev/null | awk '{print $1}' | grep -qxF "$ACTIVE_TOOLCHAIN"; then
    info "nightly toolchain in use: $ACTIVE_TOOLCHAIN"
  else
    die "The pinned toolchain '$ACTIVE_TOOLCHAIN' is not installed yet.
  Fix: rustup toolchain install $ACTIVE_TOOLCHAIN"
  fi
else
  # No rustup: cargo must itself be a nightly build for build-std to work.
  if cargo --version 2>/dev/null | grep -qi 'nightly'; then
    info "no rustup found, but cargo reports a nightly build: $(cargo --version 2>/dev/null)"
  else
    die "cargo is not a nightly build, and no rustup was found to install one. This project needs a nightly toolchain (build-std is nightly-only).
  Fix: install rustup + a nightly:
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    rustup toolchain install nightly"
  fi
fi

# (b) rust-src component (required by build-std).
if [ "$HAVE_RUSTUP" -eq 1 ] && [ -n "${ACTIVE_TOOLCHAIN:-}" ]; then
  if rustup component list --toolchain "$ACTIVE_TOOLCHAIN" 2>/dev/null | grep -qE '^rust-src[[:space:]]+\(installed\)$'; then
    info "rust-src component: installed for $ACTIVE_TOOLCHAIN (required by build-std)"
  else
    die "The 'rust-src' component is not installed for '$ACTIVE_TOOLCHAIN', but it is REQUIRED by .cargo/config.toml ([unstable] build-std = [\"alloc\", \"core\"]).
  Fix: rustup component add rust-src --toolchain $ACTIVE_TOOLCHAIN"
  fi
else
  warn "Could not query the active toolchain (no rustup). If the build below fails with a 'rust-src' or 'std' / 'library std' error, install it with:
  rustup component add rust-src"
fi

# (c) espflash — only needed to FLASH, not to build. WARN only; the build must
# still succeed without it.
if command -v espflash >/dev/null 2>&1; then
  info "espflash found: $(espflash --version 2>/dev/null || echo 'present') (used when flashing, not building)"
else
  warn "espflash was not found. That is fine for BUILDING — the build below does not invoke the flash runner.
  You will need it only to flash the firmware. Install it with:
    cargo install espflash"
fi

# ── 2. seed configs/wifi.json ONLY if absent ─────────────────────────────────
# led-core/src/wifi.rs does include_str!("../../configs/wifi.json"), so a build
# hard-fails without it. configs/wifi.json is gitignored (real credentials), but
# the tracked configs/wifi.json.example ships in every clone. We copy that
# example ONLY when the real file is missing — we never overwrite it, because on
# a machine with a working build it holds the owner's real WiFi credentials.
# We only test for existence; we never read, print, or echo its contents.
WIFI_CONFIG="configs/wifi.json"
WIFI_EXAMPLE="configs/wifi.json.example"
if [ ! -f "$WIFI_CONFIG" ]; then
  [ -f "$WIFI_EXAMPLE" ] || die "configs/wifi.json is missing and configs/wifi.json.example (the tracked example) is also missing, so there is nothing to seed it from.
  Fix: recreate configs/wifi.json.example, then re-run this script."
  cp "$WIFI_EXAMPLE" "$WIFI_CONFIG"
  hr2
  printf ' NOTICE: configs/wifi.json did not exist, so PLACEHOLDER\n'
  printf ' credentials were installed from configs/wifi.json.example.\n'
  printf ' The firmware will BUILD, but it will NOT connect to WiFi with\n'
  printf ' placeholder values. Before flashing anything that uses WiFi,\n'
  printf ' edit configs/wifi.json and put in your real credentials.\n'
  printf ' (The example is a placeholder — its contents are not shown here.)\n'
  printf '%s\n' "$RULE_DASH"
else
  info "configs/wifi.json already exists — leaving it completely untouched (this script never overwrites it; it may hold real credentials)."
fi

# ── 3. build the firmware ────────────────────────────────────────────────────
# The target is inherited from .cargo/config.toml ([build] target), so a plain
# `cargo build` builds for riscv32imac-unknown-none-elf. This uses the default
# features and does NOT invoke the espflash runner (the runner is only used by
# `cargo run` / `cargo test`), so a missing espflash does not block the build.
# Pull the target name out of the [build] section. The `^target` anchor matches
# only the `target = "..."` assignment (line 2), never the `[target....]` header.
CFG_TARGET=$(sed -nE 's/^[[:space:]]*target[[:space:]]*=[[:space:]]*"([^"]+)".*$/\1/p' .cargo/config.toml 2>/dev/null | head -n1) || CFG_TARGET=""
[ -n "$CFG_TARGET" ] || CFG_TARGET="riscv32imac-unknown-none-elf"

printf '\n[info] Building firmware for target: %s (inherited from .cargo/config.toml)\n' "$CFG_TARGET"
if cargo build; then
  ARTIFACT="target/${CFG_TARGET}/debug/esp32-c5-rgb-rs"
  if [ -f "$ARTIFACT" ]; then
    printf '\n[ok] Build succeeded. Firmware artifact (relative to repo root):\n'
    printf '      %s\n' "$ARTIFACT"
    ls -la "$ARTIFACT"
  else
    printf '\n[warn] Build succeeded but the expected artifact was not found at: %s\n' "$ARTIFACT"
    printf '       Look under: target/%s/debug/\n' "$CFG_TARGET"
  fi
else
  rc=$?
  die "Firmware build FAILED (exit ${rc}). See the cargo output above.
  If the failure happens during dependency resolution and references the
  'esp-radio' patch / the local 'forks/esp-radio' path (e.g. a missing patch
  target, or an 'esp-rom-sys' links conflict) — that is a known, separate
  issue being fixed by a different change (Cargo.toml patches esp-radio
  against the gitignored forks/ directory). It is OUT OF SCOPE for this
  script; do NOT edit Cargo.toml to 'fix' it here. Everything this script is
  responsible for (toolchain checks, wifi.json seeding) has already completed
  successfully above."
fi

# ── 4. next steps ────────────────────────────────────────────────────────────
hr
printf ' Build complete. Next steps (all commands run from the repo root):\n'
printf '%s\n' "$RULE_DASH"
printf ' Host tests (led-core):      bash scripts/test-led-core.sh\n'
printf ' Host lint/format gate:      bash scripts/check-host.sh\n'
printf ' Flash the default firmware: cargo run\n'
printf '                            (runner from .cargo/config.toml:\n'
printf '                             espflash flash --monitor --chip esp32c5)\n'
printf ' Flash an example binary:    cargo run --example led_effects_sync\n'
printf ' Explicit flash:             cargo espflash flash --example led_effects_sync --chip esp32c5 --monitor\n'
printf '%s\n' "$RULE_DASH"
printf ' NOTE: flashing requires espflash (install with `cargo install espflash`)\n'
printf ' and real credentials in configs/wifi.json before any WiFi example works.\n'
printf '%s\n' "$RULE_EQ"
