import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { applyTheme, applyThemeTypography, selectedThemeColors, selectedThemeTypography } from "../themeGallery";
import { installThemePackage, installedThemes, uninstallThemePackage } from "../themes/manager";

// GH #760: an installed package that declares typography gets its own
// "Use typography" action (independent of "Use colors"), a Typography selector
// to return to Tine's own fonts, and a non-blocking hint when its first font
// is not installed on this device.

const presence = vi.hoisted(() => ({ missing: new Set<string>() }));
vi.mock("../themes/fontAvailability", () => ({
  fontPresence: (name: string) => (presence.missing.has(name) ? "missing" : "installed"),
}));

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

const base = {
  schemaVersion: 1, version: "1.0.0", description: "A typography Settings test.", author: "Tine", license: "MIT",
  source: "https://example.invalid/theme", modes: { light: { "--ls-primary-background-color": "#fefefe" } }, screenshots: [],
};
const installTypography = () => installThemePackage({
  ...base, id: "page.tine.theme.settings-typo", name: "Plex type", apiVersion: "0.3",
  typography: { bodyFont: '"IBM Plex Sans", sans-serif', monoFont: '"IBM Plex Mono", monospace', blockLineHeight: 28 },
});
const installPlain = () => installThemePackage({ ...base, id: "page.tine.theme.settings-plain", name: "Plain colors", apiVersion: "0.2" });

async function mountAppearance() {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  openSettings("appearance");
  await tick();
  return { root, dispose };
}
const rowFor = (root: HTMLElement, name: string) =>
  [...root.querySelectorAll<HTMLElement>(".installed-theme-row")].find((row) => row.textContent?.includes(name))!;
const button = (row: HTMLElement, label: string) =>
  [...row.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label);

afterEach(async () => {
  closeSettings();
  applyTheme("");
  applyThemeTypography("");
  presence.missing.clear();
  for (const installed of installedThemes()) await uninstallThemePackage(installed.key);
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("Settings → Appearance theme typography (GH #760)", () => {
  it("offers Use typography only for a package that declares typography, separately from Use colors", async () => {
    const typo = await installTypography();
    await installPlain();
    const { root, dispose } = await mountAppearance();
    expect(button(rowFor(root, "Plain colors"), "Use typography")).toBeUndefined();
    const use = button(rowFor(root, "Plex type"), "Use typography");
    expect(use).toBeDefined();
    expect(button(rowFor(root, "Plex type"), "Use colors")).toBeDefined();
    use!.click();
    await tick();
    expect(selectedThemeTypography()).toBe(typo.key);
    expect(selectedThemeColors()).toBe("");
    expect(button(rowFor(root, "Plex type"), "Typography selected")?.disabled).toBe(true);
    dispose();
  });

  it("the Typography selector returns to Tine's own fonts and is absent without a typography package", async () => {
    await installPlain();
    let mounted = await mountAppearance();
    expect(mounted.root.querySelector('select[aria-label="Typography"]')).toBeNull();
    mounted.dispose();
    document.body.innerHTML = "";

    const typo = await installTypography();
    mounted = await mountAppearance();
    const select = mounted.root.querySelector<HTMLSelectElement>('select[aria-label="Typography"]');
    expect([...select!.options].map((option) => option.value)).toEqual(["", typo.key]);
    select!.value = typo.key;
    select!.dispatchEvent(new Event("change", { bubbles: true }));
    expect(selectedThemeTypography()).toBe(typo.key);
    select!.value = "";
    select!.dispatchEvent(new Event("change", { bubbles: true }));
    expect(selectedThemeTypography()).toBe("");
    mounted.dispose();
  });

  it("shows a non-blocking hint naming a first font that is not installed, and installation is unaffected", async () => {
    presence.missing.add("IBM Plex Sans");
    await installTypography();
    const { root, dispose } = await mountAppearance();
    const hints = [...rowFor(root, "Plex type").querySelectorAll(".installed-theme-font-hint")].map((el) => el.textContent);
    expect(hints).toEqual(["IBM Plex Sans is not installed on this device; using the fallback fonts."]);
    expect(button(rowFor(root, "Plex type"), "Use typography")?.disabled).toBe(false);
    dispose();
  });

  it("shows no hint when the fonts are installed", async () => {
    await installTypography();
    const { root, dispose } = await mountAppearance();
    expect(root.querySelector(".installed-theme-font-hint")).toBeNull();
    dispose();
  });
});
