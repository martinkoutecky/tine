import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

// Store/editor-logic tests are pure TS (no DOM). Use the node environment and
// skip the Solid JSX plugin so no jsdom is required.
export default defineConfig({
  // Build-time constants injected by Vite's `define` in prod (vite.config.ts);
  // stub them so modules that read them at import time load in tests.
  define: {
    __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00.000Z"),
    __GIT_COMMIT__: JSON.stringify(""),
    __TINE_COMMUNITY_REGISTRY__: JSON.stringify(true),
  },
  test: {
    environment: "node",
    // Tests assert behaviour, never wall time, and several do a fixed amount of
    // CPU work (whole-source scans through the TypeScript parser, 2500 wasm
    // traps, wide-grid mounts) that takes 1-3 s idle and 5-9 s when gates share
    // the host. The 5 s default is a hang detector tuned to an idle machine, so
    // it fired on load alone. A real hang still fails, after 60 s.
    testTimeout: 60_000,
    hookTimeout: 60_000,
    include: ["src/**/*.test.ts"],
    setupFiles: ["./src/testSetup.parser.ts"],
    server: { deps: { inline: ["solid-js"] } },
  },
  resolve: {
    conditions: ["browser"],
    alias: [
      { find: /^solid-js$/, replacement: fileURLToPath(new URL("./node_modules/solid-js/dist/solid.js", import.meta.url)) },
      { find: /^solid-js\/store$/, replacement: fileURLToPath(new URL("./node_modules/solid-js/store/dist/store.js", import.meta.url)) },
      { find: /^solid-js\/web$/, replacement: fileURLToPath(new URL("./node_modules/solid-js/web/dist/web.js", import.meta.url)) },
    ],
  },
  ssr: {
    resolve: { conditions: ["browser"], externalConditions: ["browser"] },
  },
});
