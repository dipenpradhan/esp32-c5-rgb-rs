# web

TypeScript + Vite single-page config UI for the on-device web server.

**Critical fact first:** the build output `web/dist/index.html` is a firmware
build input. `examples/web_server.rs` embeds it via `include_str!`. A UI
change does nothing until you (1) rebuild the UI and (2) rebuild and reflash
the firmware.

## Building

```bash
npm install
npm run build
```

`tsc && vite build`; `vite-plugin-singlefile` inlines the JS and CSS into one
self-contained `dist/index.html` with no external requests. `npm run dev`
starts the local Vite dev server if you want to work on the page in a browser.

## What is tracked

`web/.gitignore` ignores `node_modules/` and `dist/*` **except**
`!dist/index.html`. The built page is deliberately tracked — it is the file
that gets compiled into the firmware.

## Size budget

The built page is **16292 bytes**. `GET /` serves the page plus an injected
token script (72 bytes) and the HTTP header (~86 bytes) — about **16450 bytes
on the wire**. The firmware's TX buffer is **20480 bytes**
(`StaticBuf<20480>` in `examples/web_server.rs`), sized for that served
payload: 16 KiB + 4 KiB of headroom, so the page could grow roughly 25% before
the buffer needs another look. Because `send_response` loops on partial
writes, an undersized buffer only costs extra write round-trips — it can never
silently truncate the page again (that 16384-byte truncation was the 2026-08
blank-page regression). Still, check the size of `dist/index.html` after any
build: a large font, CSS block, or inlined SVG grows the served payload and
eats the headroom.

## Endpoints the UI talks to

- `GET /config` — current effect config
- `POST /config` — replace the config (validated on the device; held in RAM
  only — lost on reboot)

Sources: `src/main.ts`, `src/app.ts`, `src/style.css`, `src/index.html` (the
Vite template).
