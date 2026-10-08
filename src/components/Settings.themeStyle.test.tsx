import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { applyTheme, applyThemeColors, selectedThemeColors, selectedThemeStyle } from "../themeGallery";
import { installThemePackage, installedThemes, uninstallThemePackage } from "../themes/manager";

// Master 670cf75bb (Themes API 0.2, Settings half): Appearance offers an
// independent Style selector next to the Color scheme gallery, installed
// packages offer "Use colors" and "Use style" separately, and uninstalling a
// package clears only the roles it fills (clearThemeSelection).

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

async function installPresentationTheme() {
  return installThemePackage({
    schemaVersion: 1, id: "page.tine.theme.settings-style", name: "Editorial style", version: "1.0.0",
    apiVersion: "0.2", description: "A Settings style selector test.", author: "Tine", license: "MIT",
    source: "https://example.invalid/theme", modes: { light: { "--ls-primary-background-color": "#fefefe" } },
    presentation: { contentTypography: "editorial-serif" }, screenshots: [],
  });
}

async function mountAppearance() {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  openSettings("appearance");
  await tick();
  return { root, dispose };
}

afterEach(async () => {
  closeSettings();
  applyTheme("");
  for (const installed of installedThemes()) await uninstallThemePackage(installed.key);
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("Settings → Appearance theme Style and Color scheme (master 670cf75bb)", () => {
  it("selects a style without changing the color scheme", async () => {
    const installed = await installPresentationTheme();
    applyThemeColors("gruvbox");
    const { root, dispose } = await mountAppearance();
    const select = root.querySelector<HTMLSelectElement>('select[aria-label="Theme style"]');
    expect(select).not.toBeNull();
    expect([...select!.options].map((option) => option.value)).toEqual(["", installed.key]);
    select!.value = installed.key;
    select!.dispatchEvent(new Event("change", { bubbles: true }));
    expect(selectedThemeStyle()).toBe(installed.key);
    expect(selectedThemeColors()).toBe("gruvbox");
    expect([...root.querySelectorAll(".settings-section")].map((el) => el.textContent)).toContain("Color scheme");
    dispose();
  });

  it("offers Use colors and Use style separately for an installed presentation package", async () => {
    const installed = await installPresentationTheme();
    const { root, dispose } = await mountAppearance();
    const buttons = [...root.querySelectorAll<HTMLButtonElement>(".installed-theme-row button")];
    const useStyle = buttons.find((button) => button.textContent === "Use style");
    const useColors = buttons.find((button) => button.textContent === "Use colors");
    expect(useStyle).toBeDefined();
    expect(useColors).toBeDefined();
    useStyle!.click();
    expect(selectedThemeStyle()).toBe(installed.key);
    expect(selectedThemeColors()).toBe("");
    dispose();
  });

  it("uninstalling a package clears only the roles it fills", async () => {
    const installed = await installPresentationTheme();
    applyThemeColors("gruvbox");
    const { root, dispose } = await mountAppearance();
    root.querySelector<HTMLSelectElement>('select[aria-label="Theme style"]')!.value = installed.key;
    root.querySelector<HTMLSelectElement>('select[aria-label="Theme style"]')!.dispatchEvent(new Event("change", { bubbles: true }));
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    [...root.querySelectorAll<HTMLButtonElement>(".installed-theme-row button")]
      .find((button) => button.textContent === "Uninstall…")!.click();
    await tick(); await tick();
    expect(installedThemes()).toEqual([]);
    expect(selectedThemeStyle()).toBe("");
    expect(selectedThemeColors()).toBe("gruvbox");
    vi.restoreAllMocks();
    dispose();
  });
});
