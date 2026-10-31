import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";

/**
 * Adds a preload hint for the wasm-pack binary to the built index.html.
 *
 * The glue module requests the .wasm file only after the browser has parsed
 * it, so the preload starts the download in parallel. Rollup only knows the
 * hashed file name once the bundle is emitted, so the link is injected after
 * everything has been written to disk.
 */
function preloadWasm(): Plugin {
  let outDir = "";

  return {
    name: "preload-wasm",
    apply: "build",
    configResolved(config) {
      outDir = config.build.outDir;
    },
    writeBundle(_options, bundle) {
      const wasm = Object.keys(bundle).find((file) => file.endsWith(".wasm"));
      if (wasm === undefined) {
        return;
      }

      const indexPath = path.join(outDir, "index.html");
      const html = readFileSync(indexPath, "utf8");
      const link = `<link rel="preload" href="./${wasm}" as="fetch" type="application/wasm" crossorigin>`;
      writeFileSync(indexPath, html.replace("</head>", `${link}</head>`));
    },
  };
}

// Relative asset URLs so the bundle works at the site root and under a
// GitHub Pages project subpath (https://<user>.github.io/<repo>/).
export default defineConfig({
  base: "./",
  plugins: [preloadWasm()],
});
