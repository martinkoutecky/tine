import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { setActiveLocale } from "../i18n";
import { installThemePackage, installedThemes, uninstallThemePackage } from "../themes/manager";
import { settingEntries } from "./settingsRegistry";
import { settingLabel } from "./settingsField";
import type { ManagedPlugin } from "../plugins/manager";
import { parsePluginManifest } from "../plugins/manifest";
import { settings } from "./settingDefinitions";
import { quickShareOffered } from "../quickShareGate";

const plugins = vi.hoisted(() => ({ current: [] as ManagedPlugin[] }));
vi.mock("../plugins/manager", async original => ({
  ...await original<typeof import("../plugins/manager")>(), installedPlugins: () => plugins.current,
}));

const tick = () => new Promise(resolve => setTimeout(resolve, 0));
afterEach(async () => {
  closeSettings();
  for (const installed of installedThemes()) await uninstallThemePackage(installed.key);
  await setActiveLocale("en");
  document.body.innerHTML = "";
  localStorage.clear();
  plugins.current = [];
  vi.restoreAllMocks();
});

it.each(["en", "zh-CN"].flatMap(locale => ["Orchard dusk", "Apricot paper and ink"].map(query => [locale, query])))
("finds an installed theme by name or description and lands on its row (%s, %s, GH #866)", async (locale, query) => {
  await setActiveLocale(locale);
  await installThemePackage({
    schemaVersion: 1, id: "page.tine.theme.search-fixture", name: "Orchard dusk", version: "1.0.0",
    apiVersion: "0.2", description: "Apricot paper and ink", author: "Tine", license: "MIT",
    source: "https://example.invalid/theme", modes: { light: { "--ls-primary-background-color": "#fefefe" } }, screenshots: [],
  });
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
  try {
    openSettings("themes");
    await tick();
    expect(root.querySelector(".installed-theme-row")?.textContent).toContain("Orchard dusk");
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
      search.value = query;
      search.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await tick();
      const result = [...root.querySelectorAll<HTMLButtonElement>(".settings-search-result")]
        .find(button => button.textContent?.includes("Orchard dusk"));
      expect(result, `theme visible in Themes must be found by ${query}`).toBeDefined();
      result!.click();
      await tick();
      const target = root.querySelector(".installed-theme-row");
      expect(scroll.mock.contexts).toContain(target);
      expect(search.value).toBe("");
  } finally { dispose(); }
});

it.each(["en", "zh-CN"])("every rendered setting row is searchable by its descriptor label and lands on its ID (%s)", async locale => {
  await setActiveLocale(locale);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
  const visited = new Set<string>();
  try {
    // Include every tab currently offered by the platform/gates. Controls under
    // Advanced are mounted too. Conditional device rows have their own tests.
    for (const tab of new Set(settingEntries().map(entry => entry.tab))) {
      if (tab === "sharing" && !quickShareOffered()) continue;
      openSettings(tab);
      await tick();
      if (!root.querySelector(".settings-modal")) continue;
      const disclosure = root.querySelector<HTMLButtonElement>(".settings-advanced-toggle");
      if (disclosure?.getAttribute("aria-expanded") === "false") disclosure.click();
      await tick();
      const entries = settingEntries();
      const rows = [...root.querySelectorAll<HTMLElement>("[data-setting-id]")];
      const rowIds = rows.map(row => row.dataset.settingId!);
      expect(new Set(rowIds).size, "a setting has one mounted navigation target").toBe(rowIds.length);
      for (const row of root.querySelectorAll<HTMLElement>(".settings-field, .settings-row")) {
        if (row.querySelector(".settings-label")) expect(row.dataset.settingId, row.textContent ?? "").toBeDefined();
      }
      for (const row of rows) {
        const id = row.dataset.settingId!;
        const entry = entries.find(entry => entry.id === id);
        expect(entry, `rendered ${id} must be registered`).toBeDefined();
        if (entry!.tab === "shortcuts") continue; // shortcuts has its own command filter
        const labelNode = [...row.querySelectorAll<HTMLElement>(".settings-label, .theme-gallery-name")]
          .find(node => node.closest("[data-setting-id]") === row)?.cloneNode(true) as HTMLElement | undefined;
        labelNode?.querySelectorAll(".og-badge, .settings-hint").forEach(node => node.remove());
        const label = labelNode?.textContent?.trim() ?? settingLabel(entry!);
        expect(label, `visible label for ${id}`).toBe(settingLabel(entry!));
        const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
        search.value = label;
        search.dispatchEvent(new InputEvent("input", { bubbles: true }));
        await tick();
        const result = [...root.querySelectorAll<HTMLButtonElement>(".settings-search-result")]
          .find(button => button.dataset.settingTarget === id);
        expect(result, `${id} findable by ${label}`).toBeDefined();
        scroll.mockClear();
        result!.click();
        await tick();
        const target = [...root.querySelectorAll<HTMLElement>("[data-setting-id]")].find(row => row.dataset.settingId === id);
        expect(target, `navigation lands on ${id}`).toBeDefined();
        await vi.waitFor(() => expect(scroll.mock.contexts, `scroll lands on ${id}`).toContain(target));
        visited.add(id);
      }
    }
    expect(visited.size).toBeGreaterThan(50);
  } finally { dispose(); }
});

it("opens the correct form when plugin versions and a built-in setting share a label", async () => {
  plugins.current = ["first", "second"].map(name => ({
    manifest: parsePluginManifest({ schemaVersion: 1, apiVersion: "0.3", entry: "plugin.wasm", id: "test.colliding-versions", name,
      version: name === "first" ? "1.0.0-beta.a" : "1.0.0-beta", description: `${name} plugin`, author: "Tine", license: "MIT", source: "https://example.invalid/plugin",
      platforms: ["desktop"], capabilities: ["settings.read"], settings: [{ key: name === "first" ? "width" : "a.width", type: "boolean", label: settingLabel(settings["settings-ui-wide-mode"]), description: `${name} layout preference`, default: false }] }),
    storageId: name, storageVersion: "1", sha256: "x", selected: true, enabled: false, running: false, settings: {},
  }));
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
  try {
    openSettings("appearance");
    await tick();
    expect(new Set(settingEntries().filter(entry => entry.pluginKey).map(entry => entry.id)).size).toBe(2);
    for (const entry of settingEntries().filter(entry => entry.pluginKey)) {
      const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
      search.value = settingLabel(entry);
      search.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await tick();
      const result = [...root.querySelectorAll<HTMLButtonElement>(".settings-search-result")].find(row => row.dataset.settingTarget === entry.id);
      expect(result).toBeDefined();
      result!.click();
      await tick();
      const target = [...root.querySelectorAll<HTMLElement>("[data-setting-id]")].find(row => row.dataset.settingId === entry.id);
      expect(root.querySelector(".plugin-detail-heading h2")?.textContent).toBe(entry.keywords);
      expect(scroll.mock.contexts).toContain(target);
    }
  } finally { dispose(); }
});

it("updates search when a theme is installed or removed with Settings already open", async () => {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  try {
    openSettings("appearance");
    await tick();
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "Saffron ink";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    expect(root.querySelector(".settings-search-empty")).not.toBeNull();
    const installed = await installThemePackage({ schemaVersion: 1, apiVersion: "0.2",
      id: "page.tine.theme.live-discovery", name: "Saffron ink", version: "1.0.0", description: "An installed palette",
      author: "Tine", license: "MIT", source: "https://example.invalid/theme", modes: { light: { "--ls-primary-background-color": "#fffafa" } }, screenshots: [] });
    await tick();
    expect(root.querySelector(".settings-search-result")?.textContent).toContain("Saffron ink");
    await uninstallThemePackage(installed.key);
    await tick();
    expect(root.querySelector(".settings-search-result")).toBeNull();
    expect(root.querySelector(".settings-search-empty")).not.toBeNull();
  } finally { dispose(); }
});
