import { readFileSync, writeFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";
import wabtFactory from "wabt";

/** The wabt runtime, loaded once per config evaluation. */
let wabt: ReturnType<typeof wabtFactory> | null = null;

/**
 * Compiles WebAssembly text sources, so that a `.wat` file can be imported as
 * its wasm bytes. The SIMD probe that renderer.ts validates is written as WAT,
 * which keeps its encoding out of the sources.
 */
function webAssemblyText(): Plugin {
  return {
    name: "webassembly-text",
    async load(id) {
      const [filename] = id.split("?");
      if (!filename.endsWith(".wat")) {
        return null;
      }

      const runtime = await (wabt ??= wabtFactory());
      const module = runtime.parseWat(filename, await readFile(filename, "utf8"), { simd: true });
      const { buffer } = module.toBinary({ log: false, write_debug_names: false });
      module.destroy();

      return `export default new Uint8Array([${Array.from(buffer).join(", ")}]);`;
    },
  };
}

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
  plugins: [webAssemblyText(), preloadWasm()],
});
