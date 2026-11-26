// GH #1019: in the CodeMirror editor the `**` around a bold word sat at different heights. Inter's contextual
// alternates move an asterisk up or down by its neighbours; the rendered block and the textarea editor turned the
// feature off, each in its own rule, and the CodeMirror host (outside `.block-content`) was in neither. jsdom has no
// fonts, so this pins the one rule; a Chromium screenshot shows the glyphs.
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const read = (file: string) => fs.readFileSync(path.join(process.cwd(), file), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const sheets = ["src/styles/app.css", "src/components/cm6-editor.css"].map(read).join("\n");

describe("contextual alternates are off wherever block text is shown or edited", () => {
  const rules = [...sheets.matchAll(/([^{}]+)\{([^{}]*font-variant-ligatures:[^{}]*)\}/g)];

  it("is decided by one rule", () => {
    expect(rules).toHaveLength(1);
    expect(rules[0][2]).toMatch(/font-variant-ligatures:\s*no-contextual\s*;/);
  });

  it("covers the rendered block, the textarea editor and the CodeMirror editor", () => {
    const selectors = rules[0][1].split(",").map(s => s.trim()).sort();
    expect(selectors).toEqual([".block-content", ".block-editor", ".cm6-block-editor"]);
  });

  it("is not undone inside the CodeMirror editor, whose rules pass the font down", () => {
    const cm6 = read("src/components/cm6-editor.css");
    expect(cm6).not.toMatch(/font-variant(-ligatures)?\s*:/);
    expect(cm6).toMatch(/\.cm6-block-editor \.cm-content \{[^}]*font:\s*inherit/);
  });
});
