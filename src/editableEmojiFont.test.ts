import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("editable emoji crash guard", () => {
  it("loads a bundled monochrome emoji font", () => {
    const entry = readFileSync("src/main.tsx", "utf8");
    expect(entry).toContain('@fontsource-variable/noto-emoji/wght.css');
  });

  it("uses the bundled text face before emoji, and emoji before system fallback", () => {
    const css = readFileSync("src/styles/app.css", "utf8");
    expect(css).toMatch(
      /input:not\(\[type="checkbox"\]\):not\(\[type="radio"\]\),\s*textarea\s*\{[^}]*font-family:\s*var\(--tine-editable-font,\s*"Inter",\s*"Noto Emoji Variable",\s*var\(--ls-font-family\)\)\s*!important/s
    );
    expect(css).toMatch(
      /--tine-editable-font:[^;]*"Courier New",\s*"Noto Emoji Variable",\s*monospace/s
    );
  });

  it("does not let Noto Emoji claim ordinary keycap-base source characters", () => {
    const fontCss = readFileSync("node_modules/@fontsource-variable/noto-emoji/wght.css", "utf8");
    // The bundled emoji face advertises #, *, and 0-9, so it must follow the
    // text face in editable stacks even when they contain no emoji sequence.
    expect(fontCss).toMatch(/unicode-range:[^;]*U\+23,U\+2a,U\+30-39/i);
  });
});
