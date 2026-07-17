import { beforeEach, describe, expect, it } from "vitest";
import { CUSTOM_CSS_STYLE_ID, LS_SHIM_STYLE_ID } from "./lsShim";
import {
  THEME_GALLERY_STYLE_ID,
  applyTheme,
  ensureThemeStyle,
  selectedGalleryTheme,
  initThemeGallery,
} from "./themeGallery";
import {
  applyThemeRevocations,
  initThemePackages,
  installThemePackage,
  uninstallThemePackage,
} from "./themes/manager";

function managedStyleIds(): string[] {
  return Array.from(document.head.children)
    .map((el) => el.id)
    .filter((id) => id === LS_SHIM_STYLE_ID || id === THEME_GALLERY_STYLE_ID || id === CUSTOM_CSS_STYLE_ID);
}

describe("theme gallery style layer", () => {
  beforeEach(() => {
    document.head.innerHTML = "";
    document.body.innerHTML = "";
    applyTheme("");
    applyThemeRevocations(new Set());
  });

  it("inserts #tine-theme between the shim and custom.css", () => {
    const custom = document.createElement("style");
    custom.id = CUSTOM_CSS_STYLE_ID;
    custom.textContent = "html { --ls-primary-background-color: hotpink; }";
    document.head.appendChild(custom);

    const theme = ensureThemeStyle();

    expect(theme?.id).toBe(THEME_GALLERY_STYLE_ID);
    expect(managedStyleIds()).toEqual([
      LS_SHIM_STYLE_ID,
      THEME_GALLERY_STYLE_ID,
      CUSTOM_CSS_STYLE_ID,
    ]);
  });

  it("applies a bundled theme and Default clears the managed node", () => {
    applyTheme("nord");

    expect(selectedGalleryTheme()).toBe("nord");

    const theme = document.getElementById(THEME_GALLERY_STYLE_ID);
    if (theme) theme.textContent = "html { --scratch-theme: 1; }";

    applyTheme("");

    expect(selectedGalleryTheme()).toBe("");
    expect(theme?.textContent).toBe("");
  });

  it("applies an installed token theme without moving it after custom.css", async () => {
    const installed = await installThemePackage({
      schemaVersion: 1,
      id: "page.tine.theme.test",
      name: "Test tokens",
      version: "1.0.0",
      apiVersion: "0.1",
      description: "A test theme.",
      author: "Tine",
      license: "MIT",
      source: "https://example.invalid/theme",
      modes: { dark: { "--ls-primary-background-color": "#010203" } },
      screenshots: [],
    });
    const custom = document.createElement("style");
    custom.id = CUSTOM_CSS_STYLE_ID;
    document.head.appendChild(custom);

    applyTheme(installed.key);

    expect(selectedGalleryTheme()).toBe(installed.key);
    expect(document.getElementById(THEME_GALLERY_STYLE_ID)?.textContent).toContain("#010203");
    expect(managedStyleIds()).toEqual([LS_SHIM_STYLE_ID, THEME_GALLERY_STYLE_ID, CUSTOM_CSS_STYLE_ID]);
    await uninstallThemePackage(installed.key);
    applyTheme("");
  });

  it("refuses to apply or reinstall a theme version revoked by the signed registry", async () => {
    const manifest = {
      schemaVersion: 1 as const,
      id: "page.tine.theme.revoked",
      name: "Revoked tokens",
      version: "1.0.0",
      apiVersion: "0.1" as const,
      description: "A revoked test theme.",
      author: "Tine",
      license: "MIT",
      source: "https://example.invalid/theme",
      modes: { dark: { "--ls-primary-background-color": "#010203" } },
      screenshots: [],
    };
    const installed = await installThemePackage(manifest);
    applyTheme(installed.key);
    applyThemeRevocations(new Set([installed.key]));
    applyTheme(installed.key);

    expect(selectedGalleryTheme()).toBe("");
    expect(document.getElementById(THEME_GALLERY_STYLE_ID)?.textContent).toBe("");
    await expect(installThemePackage(manifest)).rejects.toThrow(/revoked/);
    await uninstallThemePackage(installed.key);
  });

  it("seeds cached revocations before restoring a selected installed theme", async () => {
    const installed = await installThemePackage({
      schemaVersion: 1,
      id: "page.tine.theme.startup-revoked",
      name: "Startup revoked tokens",
      version: "1.0.0",
      apiVersion: "0.1",
      description: "A startup ordering fixture.",
      author: "Tine",
      license: "MIT",
      source: "https://example.invalid/theme",
      modes: { dark: { "--ls-primary-background-color": "#010203" } },
      screenshots: [],
    });
    applyTheme(installed.key);
    applyThemeRevocations(new Set());

    await initThemePackages(new Set([installed.key]));
    await initThemeGallery();

    expect(selectedGalleryTheme()).toBe("");
    expect(document.getElementById(THEME_GALLERY_STYLE_ID)?.textContent).toBe("");
    await uninstallThemePackage(installed.key);
  });
});
