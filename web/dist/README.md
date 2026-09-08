# web/dist

The build output of the `web/` UI. One file, `index.html`, and it is **tracked
in git even though it is a build artifact** — that is deliberate, not an
oversight. The firmware embeds it at compile time; a checkout that lacks it
would not build.

## Why a build output is tracked

`examples/web_server.rs` pulls the page into the firmware binary at compile
time, not at serve time:

```rust
const INDEX_HTML: &str = include_str!("../web/dist/index.html");
// examples/web_server.rs:56
```

`include_str!` reads the file from the source tree, so `web/dist/index.html`
is a firmware **build input**. That is why it is committed rather than produced
during the firmware build. It is a Vite build artifact (see `../src/README.md`
for how it is generated), but it is also firmware source.

## The gitignore exemption

`web/.gitignore` ignores the whole directory and then negates this one file:

```
node_modules/      # web/.gitignore:1
dist/*             # web/.gitignore:2
!dist/index.html   # web/.gitignore:3
```

Everything Vite writes under `dist/` is therefore ignored except `index.html`.
The `dist/` rules live in `web/.gitignore`; the root `.gitignore` only excludes
`web/node_modules/` (`.gitignore:53`).

## Self-contained, single file

The build is configured to emit exactly one file with nothing external
referenced:

- `vite-plugin-singlefile` inlines all JS and CSS into `index.html`
  (`assetsInlineLimit: 100_000_000` and `modulePreload: false` in
  `vite.config.ts:58`–`61`).
- `index.html` carries one inlined `<style>` block and one inlined
  `<script type="module">` block; it has no `src=` / `href=` attributes and no
  `url(...)`, `https?://`, font, or image references.
- `outDir: "../dist"` with `emptyOutDir: true` clears the directory before each
  build, so after a build the only file present is `index.html`.

The page's only network requests are `fetch("/config")` calls to the device that
serves it (relative, same-origin) — its own API, not external assets.

## Served size

On disk the file is 16292 bytes. What the firmware actually sends is larger: at
serve time it injects a small `<script>window.CONFIG_TOKEN="…";</script>` before
`</head>` (`config_token_script()`, `examples/web_server.rs`) plus the HTTP
header, for ~16450 bytes on the wire. That served payload must fit the
firmware's 20480-byte TCP TX buffer; the budget and the 2026-08 partial-write
fix are documented in `web/README.md` (Size budget) and in the header comment of
`vite.config.ts`. Check the size of `index.html` after any rebuild — a large CSS
block or inlined asset grows the served payload and eats that headroom.
