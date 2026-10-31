import { readFileSync } from "node:fs";
import { expect, it, vi } from "vitest";
import { SETTING_SEARCH } from "./settingsSearch";

const inventory = readFileSync("docs/SETTINGS-INVENTORY.md", "utf8");

it("records every searchable settings key, its tab and disclosure level", () => {
  const rows = new Set(inventory.split("\n").filter(line => line.startsWith("| `")));
  const missing = SETTING_SEARCH.filter(entry => {
    const key = typeof entry.label === "string" ? entry.label : entry.label.id;
    return ![...rows].some(row => row.startsWith(`| \`${entry.tab}\` | \`${key}\` |`) && row.includes(`| ${entry.level === "advanced" ? "Advanced" : "Basic"} |`));
  }).map(entry => ({ tab: entry.tab, key: typeof entry.label === "string" ? entry.label : entry.label.id, level: entry.level ?? "basic" }));
  expect(missing).toEqual([]);
});

it("records every Settings tab, including gated Plugins and Sharing", () => {
  const source = readFileSync("src/components/Settings.tsx", "utf8");
  const tabs = source.slice(source.indexOf("const TABS:"), source.indexOf("];", source.indexOf("const TABS:")));
  const ids = [...tabs.matchAll(/id: "([a-z]+)"/g)].map(match => match[1]);
  expect(ids.length).toBeGreaterThan(0);
  expect(ids.filter(id => !inventory.includes(`<!-- tab:${id} -->`))).toEqual([]);
});

it("records gated CM6 settings even in builds where that editor is unavailable", async () => {
  vi.resetModules();
  vi.doMock("../editor/cm6Setting", () => ({ cm6EditorAvailable: true }));
  try {
    const { SETTING_SEARCH: gated } = await import("./settingsSearch");
    for (const entry of gated) {
      const key = typeof entry.label === "string" ? entry.label : entry.label.id;
      expect(inventory).toContain(`| \`${entry.tab}\` | \`${key}\` |`);
    }
  } finally {
    vi.doUnmock("../editor/cm6Setting");
    vi.resetModules();
  }
});
