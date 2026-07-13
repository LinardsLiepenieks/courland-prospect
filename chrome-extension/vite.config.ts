import { defineConfig } from "vite";
import { crx } from "@crxjs/vite-plugin";
import manifest from "./manifest.config";

export default defineConfig({
  plugins: [crx({ manifest })],
  build: {
    outDir: "dist",
    // The Tauri backend copies this dist/ into app_data and writes config.json
    // beside it, then load-unpacks from there.
    emptyOutDir: true,
  },
  server: {
    port: 5174,
    strictPort: true,
  },
});
