import { afterEach, describe, expect, it } from "vitest";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { parseThemeManifest, themeManifestCss } from "./manifest";
import { parseFontStack } from "./fontStack.mjs";
import { leadingFamilies, typographyCss } from "./typography";

// Theme API 0.3 `typography` (GH #760): the public vocabulary and its fixtures.
// Every fixture is judged by the app's parser AND by `tine-theme.mjs check`, and
// the two must agree exactly: the grammar is one module (fontStack.mjs), and this
// table is what proves the checker and the installer have not drifted.

const PLEX_SANS = '"IBM Plex Sans", -apple-system, sans-serif';
const PLEX_MONO = '"IBM Plex Mono", Menlo, monospace';

function manifestFor(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    schemaVersion: 1,
    id: "page.tine.theme.typo",
    name: "Typo",
    version: "1.0.0",
    apiVersion: "0.3",
    description: "A typography fixture.",
    author: "Tine",
    license: "MIT",
    source: "https://example.invalid/theme",
    modes: { light: { "--ls-primary-background-color": "#fff" } },
    screenshots: [],
    ...overrides,
  };
}

const VALID: Array<{ what: string; typography: unknown }> = [
  { what: "the issue's example (body, mono, title, line height)", typography: {
    bodyFont: PLEX_SANS, monoFont: PLEX_MONO, titleFont: PLEX_SANS, blockLineHeight: 28 } },
  { what: "a body font alone", typography: { bodyFont: "Georgia, serif" } },
  { what: "a mono font alone", typography: { monoFont: "Menlo, Monaco, monospace" } },
  { what: "a title font alone", typography: { titleFont: '"Iowan Old Style", Palatino, serif' } },
  { what: "a line height alone, at the lower bound", typography: { blockLineHeight: 20 } },
  { what: "a line height at the upper bound", typography: { blockLineHeight: 48 } },
  { what: "bare multi-word names and single quotes", typography: { bodyFont: "Helvetica Neue, 'Segoe UI', Arial, sans-serif" } },
  { what: "a name with digits and a dot", typography: { bodyFont: '"M PLUS 1p", "Noto Sans CJK SC", system-ui' } },
  { what: "a non-Latin family name", typography: { bodyFont: '"游ゴシック", "PingFang SC", sans-serif' } },
  { what: "a single generic family", typography: { monoFont: "monospace" } },
  { what: "eight families", typography: { bodyFont: "A, B, C, D, E, F, G, H" } },
];

const INVALID: Array<{ what: string; typography: unknown; code: string; mentions?: RegExp }> = [
  { what: "url()", typography: { bodyFont: 'url("https://evil.invalid/f.woff2")' }, code: "theme.typography-font" },
  { what: "url() hidden after a real family", typography: { bodyFont: 'Georgia, url(//evil.invalid/x)' }, code: "theme.typography-font" },
  { what: "@import", typography: { bodyFont: '@import url("x.css")' }, code: "theme.typography-font" },
  { what: "@font-face", typography: { bodyFont: "@font-face { font-family: X; src: url(x) }" }, code: "theme.typography-font" },
  { what: "a selector and braces", typography: { bodyFont: "Georgia } body { display: none" }, code: "theme.typography-font" },
  { what: "a semicolon", typography: { bodyFont: "Georgia; color: red" }, code: "theme.typography-font" },
  { what: "a declaration colon", typography: { bodyFont: "font-family: Georgia" }, code: "theme.typography-font" },
  { what: "a comment", typography: { bodyFont: "Georgia /* x */, serif" }, code: "theme.typography-font" },
  { what: "var()", typography: { bodyFont: "var(--secret), serif" }, code: "theme.typography-font" },
  { what: "an escape", typography: { bodyFont: '"Evil\\22 name"' }, code: "theme.typography-font" },
  { what: "an angle bracket", typography: { bodyFont: "<style>" }, code: "theme.typography-font" },
  { what: "a quote inside a quoted name", typography: { bodyFont: '"A"B"' }, code: "theme.typography-font" },
  { what: "an unterminated quote", typography: { bodyFont: '"Georgia, serif' }, code: "theme.typography-font" },
  { what: "a comma inside a quoted name", typography: { bodyFont: '"A, B"' }, code: "theme.typography-font" },
  { what: "!important", typography: { bodyFont: "Georgia !important" }, code: "theme.typography-font" },
  { what: "a newline", typography: { bodyFont: "Georgia,\nserif" }, code: "theme.typography-font" },
  { what: "a CSS-wide keyword", typography: { bodyFont: "inherit" }, code: "theme.typography-font" },
  { what: "a quoted CSS-wide keyword", typography: { bodyFont: '"initial", serif' }, code: "theme.typography-font" },
  { what: "an empty stack", typography: { bodyFont: "" }, code: "theme.typography-font" },
  { what: "an empty family (trailing comma)", typography: { bodyFont: "Georgia," }, code: "theme.typography-font" },
  { what: "an empty family (double comma)", typography: { bodyFont: "Georgia,, serif" }, code: "theme.typography-font" },
  { what: "an empty quoted name", typography: { bodyFont: '""' }, code: "theme.typography-font" },
  { what: "nine families", typography: { bodyFont: "A, B, C, D, E, F, G, H, I" }, code: "theme.typography-font" },
  { what: "an overlong stack", typography: { bodyFont: Array.from({ length: 8 }, (_, i) => `"${"x".repeat(30)}${i}"`).join(", ") }, code: "theme.typography-font" },
  { what: "an overlong family name", typography: { bodyFont: "x".repeat(65) }, code: "theme.typography-font" },
  { what: "a stack that is not text", typography: { bodyFont: ["Georgia"] }, code: "theme.typography-font" },
  { what: "a null stack", typography: { monoFont: null }, code: "theme.typography-font" },
  { what: "a line height below the range", typography: { blockLineHeight: 19 }, code: "theme.typography-line-height" },
  { what: "a line height above the range", typography: { blockLineHeight: 49 }, code: "theme.typography-line-height" },
  { what: "a fractional line height", typography: { blockLineHeight: 28.5 }, code: "theme.typography-line-height" },
  { what: "a line height with a unit", typography: { blockLineHeight: "28px" }, code: "theme.typography-line-height" },
  { what: "a unitless-ratio line height", typography: { blockLineHeight: 1.5 }, code: "theme.typography-line-height" },
  { what: "an unknown key", typography: { bodyFont: "Georgia", fontSize: 18 }, code: "theme.typography-field" },
  { what: "a raw CSS key", typography: { css: "body { font-family: x }" }, code: "theme.typography-field" },
  { what: "an empty block", typography: {}, code: "theme.typography" },
  { what: "an array", typography: ["Georgia"], code: "theme.typography" },
  { what: "a string", typography: "Georgia", code: "theme.typography" },
];

let scratch = "";
afterEach(() => {
  if (scratch) fs.rmSync(scratch, { recursive: true, force: true });
  scratch = "";
});

function check(manifest: Record<string, unknown>) {
  if (scratch) fs.rmSync(scratch, { recursive: true, force: true });
  scratch = fs.mkdtempSync(path.join(os.tmpdir(), "tine-theme-typo-"));
  fs.writeFileSync(path.join(scratch, "theme.json"), JSON.stringify(manifest));
  const result = spawnSync(process.execPath, ["scripts/tine-theme.mjs", "check", scratch, "--json"], { cwd: process.cwd(), encoding: "utf8" });
  return JSON.parse(result.stdout) as { status: "passed" | "failed"; errors: Array<{ code: string; message: string }> };
}
const installs = (manifest: Record<string, unknown>) => {
  try { parseThemeManifest(manifest); return true; } catch { return false; }
};

describe("theme typography vocabulary (GH #760)", () => {
  for (const { what, typography } of VALID) {
    it(`accepts ${what}, in the app and in the checker`, () => {
      const manifest = manifestFor({ typography });
      expect(installs(manifest)).toBe(true);
      const report = check(manifest);
      expect(report.errors).toEqual([]);
      expect(report.status).toBe("passed");
    });
  }

  for (const { what, typography, code } of INVALID) {
    it(`rejects ${what}, in the app and in the checker`, () => {
      const manifest = manifestFor({ typography });
      expect(installs(manifest)).toBe(false);
      const report = check(manifest);
      expect(report.status).toBe("failed");
      expect(report.errors.map((error) => error.code)).toContain(code);
    });
  }

  it("requires theme API 0.3 for typography, in the app and in the checker", () => {
    for (const apiVersion of ["0.1", "0.2"]) {
      const manifest = manifestFor({ apiVersion, typography: { blockLineHeight: 28 } });
      expect(installs(manifest)).toBe(false);
      expect(check(manifest).errors.map((e) => e.code)).toContain("theme.typography-api");
    }
  });

  it("keeps API 0.1 and 0.2 packages valid, and 0.3 accepts presentation too", () => {
    for (const apiVersion of ["0.1", "0.2", "0.3"]) {
      const manifest = manifestFor({ apiVersion });
      expect(installs(manifest), apiVersion).toBe(true);
      expect(check(manifest).status, apiVersion).toBe("passed");
    }
    const withPresentation = manifestFor({ presentation: { journalHeader: "editorial" }, typography: { blockLineHeight: 30 } });
    expect(installs(withPresentation)).toBe(true);
    expect(check(withPresentation).status).toBe("passed");
  });

  it("agrees with the published JSON schema on the vocabulary", () => {
    const schema = JSON.parse(fs.readFileSync("plugin-sdk/schema/theme.schema.json", "utf8")) as {
      properties: { apiVersion: { enum: string[] }; typography: { additionalProperties: boolean; properties: Record<string, Record<string, unknown>> } };
    };
    expect(schema.properties.apiVersion.enum).toEqual(["0.1", "0.2", "0.3"]);
    const typography = schema.properties.typography;
    expect(typography.additionalProperties).toBe(false);
    expect(Object.keys(typography.properties).sort()).toEqual(["blockLineHeight", "bodyFont", "monoFont", "titleFont"]);
    expect(typography.properties.blockLineHeight).toMatchObject({ type: "integer", minimum: 20, maximum: 48 });
    for (const field of ["bodyFont", "monoFont", "titleFont"]) {
      expect(typography.properties[field]).toMatchObject({ type: "string", minLength: 1, maxLength: 240 });
    }
  });
});

describe("font stack grammar", () => {
  it("rebuilds the CSS from the parsed families, quoting names and keeping keywords", () => {
    const parsed = parseFontStack("IBM Plex Sans, 'Segoe UI', -apple-system, BlinkMacSystemFont, SANS-SERIF");
    expect(parsed.ok && parsed.css).toBe('"IBM Plex Sans", "Segoe UI", -apple-system, BlinkMacSystemFont, sans-serif');
  });
  it("treats a quoted generic name as a family called that, not the generic", () => {
    const parsed = parseFontStack('"serif"');
    expect(parsed.ok && parsed.families[0]).toEqual({ name: "serif", kind: "named" });
  });
});

describe("typography CSS", () => {
  const typography = { bodyFont: PLEX_SANS, monoFont: PLEX_MONO, titleFont: '"Iowan Old Style", Georgia, serif', blockLineHeight: 28 };

  it("emits the public font tokens, the line-height token and the title rules, from parsed families only", () => {
    const css = typographyCss(typography);
    expect(css).toContain('--tine-content-font: "IBM Plex Sans", -apple-system, sans-serif;');
    expect(css).toContain('--tine-mono-font: "IBM Plex Mono", "Menlo", monospace;');
    expect(css).toContain("--tine-typography-line-height: 28px;");
    expect(css).toMatch(/\.page-title,[\s\S]*\.heading-text,[\s\S]*\.journal-title \{\s*font-family: "Iowan Old Style", "Georgia", serif;/);
    expect(css).toMatch(/\.block-editor\.h1,[\s\S]*\.block-editor\.h6 \{\s*--tine-editable-font: [^;]*var\(--tine-editable-emoji-font\), serif;/);
  });

  it("puts the emoji face before the final generic family of the editable stack (GH #458)", () => {
    expect(typographyCss({ bodyFont: PLEX_SANS })).toContain(
      '--tine-editable-font: "IBM Plex Sans", -apple-system, var(--tine-editable-emoji-font), sans-serif;');
    expect(typographyCss({ bodyFont: "Georgia" })).toContain('--tine-editable-font: "Georgia", var(--tine-editable-emoji-font);');
  });

  it("emits nothing for a field that is absent, and nothing at all for no typography", () => {
    expect(typographyCss(undefined)).toBe("");
    expect(typographyCss({})).toBe("");
    const css = typographyCss({ blockLineHeight: 30 });
    expect(css).toBe(":root {\n  --tine-typography-line-height: 30px;\n}");
  });

  it("never lets a hostile stored value reach the stylesheet", () => {
    const css = typographyCss({ bodyFont: "x; } body { display: none", monoFont: "url(//evil.invalid/f)", blockLineHeight: 9999 } as never);
    expect(css).toBe("");
  });

  it("does not change the color CSS of a package", () => {
    const manifest = parseThemeManifest(manifestFor({ typography }));
    expect(themeManifestCss(manifest)).not.toContain("font");
  });

  it("names the first installable family of each stack for the not-installed hint", () => {
    expect(leadingFamilies(typography)).toEqual(["IBM Plex Sans", "IBM Plex Mono", "Iowan Old Style"]);
    expect(leadingFamilies({ bodyFont: "system-ui, Georgia", monoFont: "monospace" })).toEqual([]);
    expect(leadingFamilies({ bodyFont: PLEX_SANS, titleFont: PLEX_SANS })).toEqual(["IBM Plex Sans"]);
  });
});
