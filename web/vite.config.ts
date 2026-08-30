import { defineConfig } from "vite";
import { viteSingleFile } from "vite-plugin-singlefile";
import { readFileSync } from "node:fs";

// The firmware serves this page in a single TCP write; its TX buffer is
// 16384 bytes, so an oversized page silently truncates the config UI on
// the device. Fail the build loudly instead of shipping it. (closeBundle
// only fires from a plugin in Vite 6 — not as a top-level config option —
// hence the inline plugin rather than a build option.)
const BUDGET = 16384;

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
        const size = readFileSync("dist/index.html").length;
        if (size > BUDGET) {
          console.error(`size-budget: FAIL dist/index.html ${size} > ${BUDGET} bytes (firmware TX buffer)`);
          process.exit(1);
        }
        console.log(`size-budget: OK ${size}/${BUDGET} bytes`);
      },
    },
  ],
  server: { open: true },
});
