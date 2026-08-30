import { defineConfig } from "vite";
import { viteSingleFile } from "vite-plugin-singlefile";
import { readFileSync } from "node:fs";

// ── Served-size budget ────────────────────────────────────────────────────
// The firmware (examples/web_server.rs) serves this page as:
//
//   dist/index.html  +  injected CONFIG_TOKEN script  +  HTTP header
//
// and must deliver the whole thing over its TCP socket. If the total exceeds
// the firmware's TX buffer, the page is silently truncated on the wire — the
// 2026-08 regression: page 16292 + script 72 + header 86 = 16450 bytes needed
// vs a 16384-byte TX buffer, so the last 66 bytes (end of the token script,
// `</head>`, `<body>`, and `<div id="app">`, the element the UI's JS mounts
// on) were dropped → HTTP 200 + BLANK PAGE. The old guard checked only the
// raw file (16292) against 16384 and reported OK — it validated the artifact
// on disk, not what actually goes on the wire. This guard validates the
// SERVED size instead.
//
// Every number below is named and traced to its origin:

/// The firmware's TCP transmit buffer (`TX_BUF` in examples/web_server.rs).
/// Bumped 16384 → 20480 for the 2026-08 fix; keep in sync by hand — a guard
/// that reads the Rust file would couple the build to the firmware source.
const FIRMWARE_TX_BUF = 20480;

/// The `<script>` the firmware injects before `</head>` at serve time
/// (`config_token_script()` in examples/web_server.rs):
///   `<script>window.CONFIG_TOKEN="` (29) + 32-hex token (32) + `";</script>` (11) = 72
const TOKEN_SCRIPT_BYTES = 72;

/// The HTTP header the firmware's `build_response_header` (led-core/src/http.rs)
/// emits for the "200 OK" / "text/html" response, minus the decimal digits of
/// `Content-Length`. Sum of the raw byte literals it writes, in order:
///   b"HTTP/1.1 " (9) + "200 OK" (6) + b"\r\nContent-Type: " (16)
///   + "text/html" (9) + b"\r\nContent-Length: " (18)
///   + b"\r\nConnection: close\r\n\r\n" (23) = 81
/// For a 16364-byte body the header is 81 + 5 digits = 86 bytes, matching the
/// header measured on the wire.
const HEADER_FIXED_BYTES = 81;

const headerBytesFor = (bodyBytes: number) =>
  HEADER_FIXED_BYTES + String(bodyBytes).length;

export default defineConfig({
  root: "src",
  build: {
    target: "es2020",
    minify: "esbuild",
    // Single inlined file means no external chunks, so the modulepreload
    // polyfill (~700B) is dead code; dropping it buys TX-buffer headroom.
    modulePreload: false,
    outDir: "../dist",
    emptyOutDir: true,
    assetsInlineLimit: 100_000_000,
    rollupOptions: { output: { inlineDynamicImports: true } },
  },
  plugins: [
    viteSingleFile(),
    {
      name: "size-budget",
      closeBundle() {
        // Buffer.length is the UTF-8 byte count — the unit the TX buffer
        // measures. A decoded string's .length would undercount the
        // multi-byte glyphs (✕ → · ✓) in the page.
        const pageSize = readFileSync("dist/index.html").length;
        const bodyBytes = pageSize + TOKEN_SCRIPT_BYTES;
        const headerBytes = headerBytesFor(bodyBytes);
        const servedBytes = bodyBytes + headerBytes;
        if (servedBytes > FIRMWARE_TX_BUF) {
          console.error(
            `size-budget: FAIL served page would be ${servedBytes} bytes ` +
              `(file ${pageSize} + token script ${TOKEN_SCRIPT_BYTES} + header ${headerBytes}) ` +
              `> firmware TX buffer ${FIRMWARE_TX_BUF} — page would truncate on the wire`,
          );
          process.exit(1);
        }
        console.log(
          `size-budget: OK served ${servedBytes}/${FIRMWARE_TX_BUF} bytes ` +
            `(file ${pageSize} + token script ${TOKEN_SCRIPT_BYTES} + header ${headerBytes})`,
        );
      },
    },
  ],
  server: { open: true },
});
