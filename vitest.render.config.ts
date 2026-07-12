import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";
import { fileURLToPath } from "node:url";

// Separate config for the AST renderer tests: these import the .tsx render
// components and mount them into a jsdom DOM (client mode), so they need the
// Solid plugin + a DOM environment (the main vitest config is pure-TS / node).
export default defineConfig({
  plugins: [solid()],
  // Build-time constants injected by Vite's `define` in prod (vite.config.ts);
  // stub them so components that read them can be mounted in tests.
  define: {
    __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00.000Z"),
    __GIT_COMMIT__: JSON.stringify(""),
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.tsx"],
    server: { deps: { inline: ["solid-js"] } },
  },
  resolve: {
    conditions: ["browser"],
    alias: [
      { find: /^solid-js$/, replacement: fileURLToPath(new URL("./node_modules/solid-js/dist/dev.js", import.meta.url)) },
      { find: /^solid-js\/store$/, replacement: fileURLToPath(new URL("./node_modules/solid-js/store/dist/dev.js", import.meta.url)) },
      { find: /^solid-js\/web$/, replacement: fileURLToPath(new URL("./node_modules/solid-js/web/dist/dev.js", import.meta.url)) },
    ],
  },
  ssr: {
    resolve: { conditions: ["browser"], externalConditions: ["browser"] },
  },
});
