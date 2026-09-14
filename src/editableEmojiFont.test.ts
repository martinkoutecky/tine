import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { editableEmojiPlatform } from "./editableEmoji";

describe("editable emoji crash guard", () => {
  it("loads a bundled monochrome emoji font", () => {
    const entry = readFileSync("src/main.tsx", "utf8");
    expect(entry).toContain('@fontsource-variable/noto-emoji/wght.css');
  });

  it.each(["src/main.tsx", "src/capture.tsx"])("initializes the same font policy in %s", (path) => {
    const entry = readFileSync(path, "utf8");
    expect(entry).toContain("@fontsource-variable/noto-emoji/wght.css");
    expect(entry).toContain("./styles/inter.css");
    expect(entry).toContain("installEditableEmojiPlatform();");
  });

  it("uses platform color emoji before the safe fallback except on desktop Linux", () => {
    const css = readFileSync("src/styles/app.css", "utf8");
    expect(css).toContain('--tine-editable-emoji-font: "Noto Emoji Variable"');
    expect(css).toContain('--tine-editable-emoji-font: "Segoe UI Emoji", "Noto Emoji Variable"');
    expect(css).toContain('--tine-editable-emoji-font: "Apple Color Emoji", "Noto Emoji Variable"');
    expect(css).toMatch(/font-family:\s*var\(--tine-editable-font,\s*"Inter",\s*var\(--tine-editable-emoji-font\)/s);
    expect(css).toMatch(/--tine-editable-font:[^;]*"Courier New",\s*var\(--tine-editable-emoji-font\),\s*monospace/s);

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
