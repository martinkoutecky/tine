import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import {
  THEME_GALLERY_STYLE_ID,
  applyTheme,
  applyThemeColors,
  applyThemeStyle,
  applyThemeTypography,
  clearThemeSelection,
  initThemeGallery,
  selectedThemeColors,
  selectedThemeStyle,
  selectedThemeTypography,
} from "./themeGallery";
import {
  applyThemeRevocations,
  installThemePackage,
  installedThemes,
  uninstallThemePackage,
} from "./themes/manager";

// Theme API 0.3 typography (GH #760): the third, independent composition role.

const themeCss = () => document.getElementById(THEME_GALLERY_STYLE_ID)?.textContent ?? "";

function typographyTheme(over: Record<string, unknown> = {}) {
  return installThemePackage({
    schemaVersion: 1, id: "page.tine.theme.plex", name: "Plex", version: "1.0.0", apiVersion: "0.3",
    description: "Typography composition fixture.", author: "Tine", license: "MIT", source: "https://example.invalid/theme",
    modes: { light: { "--ls-primary-background-color": "#fefefe" } },
    typography: { bodyFont: '"IBM Plex Sans", sans-serif', monoFont: '"IBM Plex Mono", monospace', blockLineHeight: 28 },
    screenshots: [],
    ...over,
  });
}

function colorsTheme(id: string, color: string) {
  return installThemePackage({
    schemaVersion: 1, id, name: id, version: "1.0.0", apiVersion: "0.2",
    description: "Colors only.", author: "Tine", license: "MIT", source: "https://example.invalid/theme",
    modes: { light: { "--ls-primary-background-color": color } }, screenshots: [],
  });
}

beforeEach(async () => {
  document.head.innerHTML = "";
  document.body.innerHTML = "";
  applyThemeRevocations(new Set());
  applyTheme("");
  applyThemeTypography("");
  for (const installed of installedThemes()) await uninstallThemePackage(installed.key);
});

describe("theme typography in the composition", () => {
  it("applies the declared fonts and line height, and reverting restores the previous look exactly", async () => {
    const installed = await typographyTheme();
    expect(themeCss()).toBe("");
    const before = document.head.innerHTML;

    applyThemeTypography(installed.key);
    expect(selectedThemeTypography()).toBe(installed.key);
    expect(themeCss()).toContain('--tine-content-font: "IBM Plex Sans", sans-serif;');
    expect(themeCss()).toContain('--tine-mono-font: "IBM Plex Mono", monospace;');
    expect(themeCss()).toContain("--tine-typography-line-height: 28px;");
    // Typography alone adds no colors and no presentation attributes.
    expect(themeCss()).not.toContain("--ls-primary-background-color");
    expect(selectedThemeColors()).toBe("");
    expect(document.documentElement.hasAttribute("data-theme-content-typography")).toBe(false);

    applyThemeTypography("");
    expect(selectedThemeTypography()).toBe("");
    expect(themeCss()).toBe("");
    expect(document.head.innerHTML.replace(/\s+/g, " ")).toBe(before.replace(/\s+/g, " "));
  });

  it("is independent of colors and style: each role keeps its own source", async () => {
    const installed = await typographyTheme();
    const red = await colorsTheme("page.tine.theme.red", "#aa0000");
    const blue = await colorsTheme("page.tine.theme.blue", "#0000bb");
    applyThemeColors(red.key);
    applyThemeTypography(installed.key);
    expect(themeCss()).toContain("#aa0000");
    expect(themeCss()).toContain("--tine-content-font");
    expect(selectedThemeColors()).toBe(red.key);

    applyThemeColors(blue.key);
    expect(themeCss()).toContain("#0000bb");
    expect(themeCss()).not.toContain("#aa0000");
    expect(themeCss()).toContain("--tine-content-font");
    expect(selectedThemeTypography()).toBe(installed.key);

    applyThemeColors("");
    expect(themeCss()).not.toContain("#0000bb");
    expect(themeCss()).toContain("--tine-content-font");

    // "Use colors" of the same package does not drag typography along, nor the reverse.
    applyThemeTypography("");
    applyThemeColors(installed.key);
    expect(selectedThemeTypography()).toBe("");
    expect(themeCss()).toContain("#fefefe");
    expect(themeCss()).not.toContain("--tine-content-font");
  });

  it("the compatibility preset applyTheme leaves the chosen typography alone", async () => {
    const installed = await typographyTheme();
    applyThemeTypography(installed.key);
    applyTheme("nord");
    expect(selectedThemeTypography()).toBe(installed.key);
    applyTheme("");
    expect(selectedThemeTypography()).toBe(installed.key);
  });

  it("composes with an editorial presentation: both stay selected", async () => {
    const editorial = await installThemePackage({
      schemaVersion: 1, id: "page.tine.theme.ed", name: "Ed", version: "1.0.0", apiVersion: "0.2",
      description: "Presentation fixture.", author: "Tine", license: "MIT", source: "https://example.invalid/theme",
      modes: { light: { "--ls-primary-background-color": "#fefefe" } },
      presentation: { contentTypography: "editorial-serif" }, screenshots: [],
    });
    const typo = await typographyTheme();
    applyThemeStyle(editorial.key);
    applyThemeTypography(typo.key);
    expect(selectedThemeStyle()).toBe(editorial.key);
    expect(selectedThemeTypography()).toBe(typo.key);
    expect(document.documentElement.getAttribute("data-theme-content-typography")).toBe("editorial-serif");
  });

  it("only a package that declares typography can fill the role", async () => {
    const colorsOnly = await installThemePackage({
      schemaVersion: 1, id: "page.tine.theme.plain", name: "Plain", version: "1.0.0", apiVersion: "0.2",
      description: "No typography.", author: "Tine", license: "MIT", source: "https://example.invalid/theme",
      modes: { light: { "--ls-primary-background-color": "#fefefe" } }, screenshots: [],
    });
    applyThemeTypography(colorsOnly.key);
    expect(selectedThemeTypography()).toBe("");
    applyThemeTypography("nord");
    expect(selectedThemeTypography()).toBe("");
    applyThemeTypography("page.tine.theme.missing@9.9.9");
    expect(selectedThemeTypography()).toBe("");
  });

  it("uninstalling the package reverts its typography and keeps the other roles", async () => {
    const installed = await typographyTheme();
    const red = await colorsTheme("page.tine.theme.red", "#aa0000");
    applyThemeColors(red.key);
    applyThemeTypography(installed.key);
    clearThemeSelection(installed.key);
    await uninstallThemePackage(installed.key);
    expect(selectedThemeTypography()).toBe("");
    expect(selectedThemeColors()).toBe(red.key);
    expect(themeCss()).not.toContain("--tine-content-font");
    expect(themeCss()).toContain("#aa0000");
  });

  it("a revoked package's typography is dropped", async () => {
    const installed = await typographyTheme();
    applyThemeTypography(installed.key);
    applyThemeRevocations(new Set([installed.key]));
    applyThemeTypography(installed.key);
    expect(selectedThemeTypography()).toBe("");
    expect(themeCss()).toBe("");
  });

  it("persists typography in the one composition record only when chosen, and restores it", async () => {
    const installed = await typographyTheme();
    const set = vi.spyOn(backend(), "setAppString");
    applyThemeColors("nord");
    await vi.waitFor(() => expect(set).toHaveBeenCalledWith("theme.composition.v1", JSON.stringify({ style: "", colors: "nord" })));
    applyThemeTypography(installed.key);
    await vi.waitFor(() => expect(set).toHaveBeenCalledWith(
      "theme.composition.v1", JSON.stringify({ style: "", colors: "nord", typography: installed.key })));
    set.mockRestore();

    applyThemeTypography("");
    applyThemeColors("");
    await new Promise((resolve) => setTimeout(resolve, 20));
    const get = vi.spyOn(backend(), "getAppString").mockImplementation(async (key, fallback) =>
      key === "theme.composition.v1" ? JSON.stringify({ style: "", colors: "nord", typography: installed.key }) : fallback);
    await initThemeGallery();
    expect(selectedThemeTypography()).toBe(installed.key);
    expect(selectedThemeColors()).toBe("nord");
    expect(themeCss()).toContain("--tine-content-font");
    get.mockRestore();
  });

  it("reads a composition saved before typography existed", async () => {
    const get = vi.spyOn(backend(), "getAppString").mockImplementation(async (key, fallback) =>
      key === "theme.composition.v1" ? JSON.stringify({ style: "", colors: "gruvbox" }) : fallback);
    await initThemeGallery();
    expect(selectedThemeColors()).toBe("gruvbox");
    expect(selectedThemeTypography()).toBe("");
    get.mockRestore();
  });
});
