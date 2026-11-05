import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import solid from "vite-plugin-solid";

// Load the actual TypeScript registries without starting the app, reading its
// preferences, or running the production Vite build hooks.
const root = fileURLToPath(new URL("..", import.meta.url));
const server = await createServer({ root, configFile: false, plugins: [solid({ ssr: true })],
  server: { middlewareMode: true }, appType: "custom", logLevel: "error",
  define: { __TINE_COMMUNITY_REGISTRY__: "true" },
});
try {
  const { generatedSettingsInventory } = await server.ssrLoadModule("/src/components/settingsInventory.ts");
  const expected = generatedSettingsInventory();
  const destination = new URL("../docs/SETTINGS-INVENTORY.md", import.meta.url);
  if (process.argv.includes("--check")) {
    if (readFileSync(destination, "utf8") !== expected) throw new Error("Settings inventory is stale: run node scripts/generate-settings-inventory.mjs");
    console.log("Settings inventory is current");
  } else {
    writeFileSync(destination, expected);
    console.log("Generated docs/SETTINGS-INVENTORY.md");
  }
} finally { await server.close(); }
