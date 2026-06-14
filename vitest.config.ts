import { defineConfig } from "vitest/config";

// Store/editor-logic tests are pure TS (no DOM). Use the node environment and
// skip the Solid JSX plugin so no jsdom is required.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
  resolve: {
    conditions: ["browser"],
  },
});
