import { defineConfig } from "vite";
import { viteSingleFile } from "vite-plugin-singlefile";

// vite-plugin-singlefile requires a single entry per build (it forces
// `inlineDynamicImports`, which Rollup forbids for multi-entry). The npm
// `build` script clears dist/ once, then invokes this config per widget
// via `WIDGET=keypad|step`. `emptyOutDir: false` so per-entry builds
// don't clobber siblings.
const widget = process.env.WIDGET || "keypad";

export default defineConfig({
  plugins: [viteSingleFile()],
  build: {
    target: "esnext",
    cssMinify: true,
    minify: true,
    rollupOptions: {
      input: `${widget}.html`,
    },
    outDir: "dist",
    emptyOutDir: false,
  },
});
