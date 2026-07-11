import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("editable emoji crash guard", () => {
  it("loads a bundled monochrome emoji font", () => {
    const entry = readFileSync("src/main.tsx", "utf8");
    expect(entry).toContain('@fontsource-variable/noto-emoji/wght.css');
  });

  it("puts the monochrome font ahead of fallback fonts on every raw-text control", () => {
    const css = readFileSync("src/styles/app.css", "utf8");
    expect(css).toMatch(
      /input:not\(\[type="checkbox"\]\):not\(\[type="radio"\]\),\s*textarea\s*\{[^}]*font-family:\s*"Noto Emoji Variable",\s*var\(--tine-editable-fallback,\s*var\(--ls-font-family\)\)\s*!important/s
    );
  });
});
