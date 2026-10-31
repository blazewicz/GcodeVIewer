import { defineConfig } from "vite";

// Relative asset URLs so the bundle works at the site root and under a
// GitHub Pages project subpath (https://<user>.github.io/<repo>/).
export default defineConfig({
  base: "./",
});
