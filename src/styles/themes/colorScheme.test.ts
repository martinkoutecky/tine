import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { galleryThemes } from "./index";
import { parseThemeManifest, themeManifestCss } from "../../themes/manifest";

// Native controls (checkboxes, number spinners, scrollbars) follow `color-scheme`. Tine's only
// dark signal is the mode attribute `html[data-theme="dark"]` (themePreference.ts), and the
// stock rule for that mode sets `color-scheme: dark` (theme.css). Every theme, bundled or
// installed, layers its colours on that same mode selector, so a dark theme inherits the
// native dark controls, provided no theme can replace the property or leave the mode selector.
// A palette's own luminance is deliberately not a signal.

const themeCss = readFileSync("src/styles/theme.css", "utf8");
const modeBlock = (css: string, mode: "light" | "dark") =>
  css.match(new RegExp(`html\\[data-theme="${mode}"\\]\\s*\\{([^}]*)\\}`))?.[1] ?? "";

const installed = parseThemeManifest({
  schemaVersion: 1, id: "test.dark-only", name: "Dark only", version: "1.0.0", apiVersion: "0.2",
  description: "A dark theme", author: "test", license: "MIT", source: "https://example.com/t",
  modes: { dark: { "--ls-primary-background-color": "#101010", "--ls-primary-text-color": "#eeeeee" } },
  screenshots: [],
});

describe("native controls follow the dark mode for every theme", () => {
  it("the stock dark mode sets color-scheme: dark, and light sets light", () => {
    expect(modeBlock(themeCss, "dark")).toMatch(/color-scheme:\s*dark\s*;/);
    expect(modeBlock(themeCss, "light")).toMatch(/color-scheme:\s*light\s*;/);
  });

  const sheets: [string, string][] = [
    ...galleryThemes.map((theme): [string, string] => [theme.id, readFileSync(`src/styles/themes/${theme.id}.css`, "utf8")]),
    ["installed manifest", themeManifestCss(installed)],
  ];
  for (const [name, css] of sheets) {
    it(`${name} keys its palette on the light/dark mode and never overrides color-scheme`, () => {
      const code = css.replace(/\/\*[\s\S]*?\*\//g, "");
      expect(code).not.toMatch(/color-scheme/);
      // Whatever else a theme styles, its palette sits on the mode selectors: the ones the
      // stock `color-scheme` rule is keyed on. No other dark/light attribute is introduced.
      const attributes = [...code.matchAll(/data-theme\s*=\s*"([^"]*)"/g)].map((m) => m[1]);
      expect(attributes.length).toBeGreaterThan(0);
      for (const value of attributes) expect(["light", "dark"]).toContain(value);
    });
  }

  it("each bundled theme with a dark mode restyles the very selector that carries color-scheme: dark", () => {
    for (const theme of galleryThemes.filter((t) => t.modes.includes("dark"))) {
      expect(modeBlock(readFileSync(`src/styles/themes/${theme.id}.css`, "utf8"), "dark"), theme.id).not.toBe("");
    }
  });
});
