import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { editableEmojiPlatform } from "./editableEmoji";

// Master 8ce04ad31 + 4f3e1f90d (#458) at master HEAD. og keeps the editable
// font policy in src/styles/editableEmoji.css (out of app.css, size ratchet).
const policyCss = () => readFileSync("src/styles/editableEmoji.css", "utf8");

describe("editable emoji crash guard", () => {
  it("loads a bundled monochrome emoji font", () => {
    const entry = readFileSync("src/main.tsx", "utf8");
    expect(entry).toContain('@fontsource-variable/noto-emoji/wght.css');
  });

  it.each(["src/main.tsx", "src/capture.tsx"])("initializes the same font policy in %s", (path) => {
    const entry = readFileSync(path, "utf8");
    expect(entry).toContain("@fontsource-variable/noto-emoji/wght.css");
    expect(entry).toContain("./styles/inter.css");
    expect(entry).toContain("./styles/editableEmoji.css");
    expect(entry).toContain("installEditableEmojiPlatform();");
  });

  it("uses platform color emoji before the safe fallback except on desktop Linux", () => {
    const css = policyCss();
    expect(css).toContain('--tine-editable-emoji-font: "Noto Emoji Variable"');
    expect(css).toContain('--tine-editable-emoji-font: "Segoe UI Emoji", "Noto Emoji Variable"');
    expect(css).toContain('--tine-editable-emoji-font: "Apple Color Emoji", "Noto Emoji Variable"');
    // A published export is read in ordinary browsers: color first.
    expect(css).toMatch(/html\[data-editable-emoji="published"\]\s*\{\s*--tine-editable-emoji-font:\s*"Apple Color Emoji",\s*"Segoe UI Emoji",\s*"Noto Color Emoji",\s*"Noto Emoji Variable"/s);
    expect(css).toMatch(/\.emoji-native,\s*input:not\(\[type="checkbox"\]\):not\(\[type="radio"\]\),\s*textarea\s*\{[^}]*font-family:\s*var\(--tine-editable-font,\s*"Inter",\s*var\(--tine-editable-emoji-font\),\s*var\(--ls-font-family\)\)\s*!important/s);
    expect(css).toMatch(/--tine-editable-font:[^;]*"Courier New",\s*var\(--tine-editable-emoji-font\),\s*monospace/s);
    expect(readFileSync("src/styles/themePresentation.css", "utf8")).toMatch(/Georgia,\s*var\(--tine-editable-emoji-font\),\s*serif/);

    expect(editableEmojiPlatform("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe("windows");
    expect(editableEmojiPlatform("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15")).toBe("safe-monochrome");
    expect(editableEmojiPlatform("Mozilla/5.0 (Linux; Android 15)")).toBe("android");
    expect(editableEmojiPlatform("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")).toBe("apple");
  });

  it("does not let Noto Emoji claim ordinary keycap-base source characters", () => {
    const fontCss = readFileSync("node_modules/@fontsource-variable/noto-emoji/wght.css", "utf8");
    // The bundled emoji face advertises #, *, and 0-9, so it must follow the
    // text face in editable stacks even when they contain no emoji sequence.
    expect(fontCss).toMatch(/unicode-range:[^;]*U\+23,U\+2a,U\+30-39/i);
  });
});
