import { describe, expect, it } from "vitest";
import { parseThemeManifest, themeManifestCss, themeVersionKey } from "./manifest";

const valid = {
  schemaVersion: 1,
  id: "page.tine.theme.example",
  name: "Example",
  version: "1.0.0",
  apiVersion: "0.1",
  description: "A safe token theme.",
  author: "Theme Author",
  license: "MIT",
  source: "https://example.invalid/theme",
  modes: {
    light: { "--ls-primary-background-color": "#ffffff", "--ls-primary-text-color": "rgb(10 20 30)" },
    dark: { "--ls-primary-background-color": "#101010", "--ls-primary-text-color": "#eeeeee" },
  },
  screenshots: ["https://example.invalid/screenshot.png"],
};

describe("theme manifests", () => {
  it("parses inert semantic tokens and emits selector-bounded CSS", () => {
    const manifest = parseThemeManifest(valid);
    expect(themeVersionKey(manifest)).toBe("page.tine.theme.example@1.0.0");
    expect(themeManifestCss(manifest)).toContain('html[data-theme="dark"]');
    expect(themeManifestCss(manifest)).toContain("--ls-primary-text-color: #eeeeee;");
  });

  it("rejects selectors, unknown tokens, imports, URLs, and variable indirection", () => {
    for (const [token, value] of [
      ["--unknown", "#fff"],
      ["--ls-primary-background-color", "url(https://evil.invalid/x)"],
      ["--ls-primary-background-color", "var(--secret)"],
      ["--ls-primary-background-color", "#fff; } body { display:none"],
    ]) {
      expect(() => parseThemeManifest({ ...valid, modes: { light: { [token]: value } } })).toThrow();
    }
  });

  it("requires public source and screenshot URLs", () => {
    expect(() => parseThemeManifest({ ...valid, source: "file:///tmp/theme" })).toThrow(/https/);
    expect(() => parseThemeManifest({ ...valid, screenshots: ["data:text/html,bad"] })).toThrow(/https/);
  });
});
