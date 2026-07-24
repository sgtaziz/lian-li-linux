import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath, URL } from "node:url";

// Tauri expects a fixed port; the dev server runs on 1420.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [vue()],
  clearScreen: false,
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  // Emit assets relative to dist root so Tauri can embed them.
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
}));
