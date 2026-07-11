import { defineConfig } from "vitest/config";

// Store/editor-logic tests are pure TS (no DOM). Use the node environment and
// skip the Solid JSX plugin so no jsdom is required.
export default defineConfig({
  // Build-time constants injected by Vite's `define` in prod (vite.config.ts);
  // stub them so modules that read them at import time load in tests.
  define: {
    __BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00.000Z"),
    __GIT_COMMIT__: JSON.stringify(""),
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
    server: { deps: { inline: [/^solid-js(?:\/|$)/] } },
  },
  resolve: {
    conditions: ["browser"],
  },
  ssr: {
    resolve: { conditions: ["browser"], externalConditions: ["browser"] },
  },
});
