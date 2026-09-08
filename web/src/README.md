# web/src

The TypeScript + Vite source for the single-page config UI that the on-device
web server (`examples/web_server.rs`) serves. It is the *source*; the file the
firmware actually embeds is the build output `web/dist/index.html` (see
`dist/README.md`). A UI change reaches the device only after rebuilding this
and rebuilding/reflashing the firmware.

## The files

- **`index.html`** — the Vite entry template. Loads `./main.ts` as a module and
  `./style.css`; contains the empty `<div id="app">` the UI mounts on.
- **`main.ts`** — the entry point. Imports `style.css` and `app.ts`, instantiates
  `App` on the `#app` element, and calls `render()` then `loadFromDevice()`.
- **`app.ts`** — the UI. `App` renders the preview, the effect list, and the
  config-JSON panel, and talks to the device over two endpoints:
  `GET /config` (`loadFromDevice`, `web/src/app.ts:302`) and `POST /config`
  (`saveToDevice`, `web/src/app.ts:280`). `POST /config` is auth-gated: it
  sends the device's per-config token in the `X-Config-Token` header, reading
  it from `window.CONFIG_TOKEN`, which the firmware injects into the served
  page at serve time (`web/src/app.ts:271`–`280`). Device JSON is treated as
  untrusted input and sanitized/clamped against limits the firmware shares
  (`MAX_EFFECTS 32`, `MAX_COLORS 64`, `MIN_STEPS 1`, `MAX_STEPS 64`,
  `MAX_MS 60000`; `web/src/app.ts:30`–`34`).
- **`style.css`** — the stylesheet, inlined into the build by Vite.

## How it relates to `web/dist/index.html`

`npm run build` runs `tsc && vite build`. Vite (configured in
`web/vite.config.ts`) builds for `es2020` into `../dist` and the
`vite-plugin-singlefile` plugin inlines all JS and CSS into a single
self-contained `dist/index.html` — no external requests at serve time. That
tracked `dist/index.html` is what `examples/web_server.rs` embeds with
`include_str!` (`examples/web_server.rs:56`). `tsconfig.json` sets `noEmit`
true; the type-check step is separate from the Vite bundle.
