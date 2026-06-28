import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";

// Separate config for the AST renderer tests: these import the .tsx render
// components and mount them into a jsdom DOM (client mode), so they need the
// Solid plugin + a DOM environment (the main vitest config is pure-TS / node).
export default defineConfig({
  plugins: [solid()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.tsx"],
  },
  resolve: {
    conditions: ["browser"],
  },
});
