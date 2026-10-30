// CM6-LINEHEIGHT (Martin, 2026-10-30): entering the experimental CodeMirror editor shifted every wrapped line by a
// pixel or two. The editor host sits outside `.block-content`, so it inherited the body's unitless 1.5 (24px) instead of
// the block's 26px line, and its content had no 2px vertical padding. jsdom has no layout, so this pins the declared
// metrics against the rendered block's own rule; scripts/shot-cm6-line-height.mjs measures the real engines.
import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const read = (file: string) => fs.readFileSync(path.join(process.cwd(), file), "utf8");
const app = read("src/styles/app.css");
const cm6 = read("src/components/cm6-editor.css");

function ruleBody(css: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\:]/g, "\\$&");
  const m = css.match(new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`));
  if (!m) throw new Error(`no rule for ${selector}`);
  return m[1].replace(/\/\*[\s\S]*?\*\//g, "");
}
const decl = (body: string, property: string) =>
  body.match(new RegExp(`(?:^|[;\\s])${property}:\\s*([^;]+?)\\s*(?:;|$)`))?.[1].replace(/\s+/g, " ");

describe("CM6 block editor line metrics match the rendered block", () => {
  const block = ruleBody(app, ".block-content");

  it("sets the block's own line-height on the editor host so CodeMirror inherits it", () => {
    expect(decl(block, "line-height")).toBe("var(--ls-block-line-height)");
    const host = ruleBody(cm6, ".cm6-block-editor");
    expect(decl(host, "line-height")).toBe(decl(block, "line-height"));
  });

  it("never lets the body's unitless ratio reach the editor lines", () => {
    // `font: inherit` resets line-height to inherit; a ratio or `normal` here would reintroduce the 24px lines.
    for (const selector of [".cm6-block-editor .cm-editor", ".cm6-block-editor .cm-scroller", ".cm6-block-editor .cm-content"]) {
      const body = ruleBody(cm6, selector);
      expect(decl(body, "line-height") ?? "inherit").toBe("inherit");
    }
  });

  it("gives the content the block's vertical padding and one-line floor", () => {
    const content = ruleBody(cm6, ".cm6-block-editor:not(:has(.code-edit)) .cm-content");
    expect(decl(content, "padding")).toBe(decl(block, "padding"));
    expect(decl(content, "min-height")).toBe(decl(block, "min-height"));
  });

  it("leaves the code view's own card metrics alone", () => {
    const card = ruleBody(cm6, ".cm6-block-editor:has(.code-edit) .cm-editor");
    expect(decl(card, "line-height")).toBe("1.5");
  });

  // CM6-CODEJUMP: the code card's box is the textarea card's, so a code block is as tall in the CM6 editor as in the
  // textarea and the view; its language label and line numbers (positioned over the card) must be painted above the
  // CM6 card, which is itself positioned and comes later in the DOM.
  it("gives the code card the textarea card's box", () => {
    const textarea = ruleBody(app, ".block-editor.code-edit");
    const card = ruleBody(cm6, ".cm6-block-editor:has(.code-edit) .cm-editor");
    for (const property of ["padding", "margin", "border", "border-radius"]) expect(decl(card, property), property).toBe(decl(textarea, property));
  });

  it("paints the language label and the line numbers above the CM6 card", () => {
    const over = ruleBody(cm6, ".cm6-block-editor .code-editor-region > :is(.code-gutter, .code-language)");
    expect(Number(decl(over, "z-index"))).toBeGreaterThanOrEqual(1);
  });
});
