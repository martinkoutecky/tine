import { beforeEach, describe, expect, it } from "vitest";
import { CUSTOM_CSS_STYLE_ID, LS_SHIM_STYLE_ID } from "./lsShim";
import {
  THEME_GALLERY_STYLE_ID,
  applyTheme,
  ensureThemeStyle,
  selectedGalleryTheme,
} from "./themeGallery";

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
});
