import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { generatedSettingsInventory } from "./settingsInventory";
import { settings } from "./settingDefinitions";

it("keeps the generated settings inventory current, including gated descriptors", () => {
  expect(readFileSync("docs/SETTINGS-INVENTORY.md", "utf8")).toBe(generatedSettingsInventory());
  expect(generatedSettingsInventory()).toContain(settings["settings-cm6-label"].id);
});
