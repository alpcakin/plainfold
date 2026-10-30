import { defineConfig } from "vite";

// Tauri loads the dev server on a fixed port. `fs.allow` lets the browser preview read a model file
// from elsewhere in the repository (`?preview=/@fs/<absolute path to model.json>`). The shell's Rust build
// folder (several GB) is no part of the view, so it is not watched.
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true, fs: { allow: [".."] }, watch: { ignored: ["**/src-tauri/target/**"] } },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: { target: ["es2022", "safari16", "chrome110"], outDir: "dist", emptyOutDir: true },
});
