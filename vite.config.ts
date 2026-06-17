import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Build timestamp, stamped at bundle time and shown in Settings so it's easy to
// confirm the running binary is the latest (vs. a stale Syncthing copy).
const BUILD_TIME = new Date().toISOString();

// Tauri expects a fixed port and serves the built assets from dist/.
export default defineConfig({
  plugins: [solid()],
  define: {
    __BUILD_TIME__: JSON.stringify(BUILD_TIME),
  },
  clearScreen: false,
  server: {
    port: 5181,
    strictPort: true,
  },
  build: {
    target: "esnext",
    outDir: "dist",
  },
});
