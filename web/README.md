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

The built page is **16163 bytes**. The firmware's TCP TX buffer is **16384
bytes** (`StaticBuf<16384>` in `examples/web_server.rs`), leaving only ~221
bytes of headroom for the HTTP framing around the page. Adding a font, a large
CSS block or an inlined SVG can push the page past what fits a single-write
delivery. Check the size of `dist/index.html` after any build.

## Endpoints the UI talks to

- `GET /config` — current effect config
- `POST /config` — replace the config (validated on the device; held in RAM
  only — lost on reboot)

Sources: `src/main.ts`, `src/app.ts`, `src/style.css`, `src/index.html` (the
Vite template).
