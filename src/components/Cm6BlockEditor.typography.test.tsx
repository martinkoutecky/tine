import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { initParser } from "../render/parse";
import { node, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { startEditing, endEdit } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { setTypographyModeSig, typographyMode } from "../ui";

// clicking into a block must not change how `--` or `->` look. With typographic replacements on
// (the default), the editor draws the view's glyphs over the ASCII the file keeps, and shows the ASCII where the caret is.
let dispose: (() => void) | undefined;
const initial = typographyMode();
beforeAll(() => {
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  return initParser();
});
afterEach(() => { dispose?.(); dispose = undefined; endEdit("page-navigation"); resetStore(); setTypographyModeSig(initial); document.body.innerHTML = ""; });

function mount(raw: string, caret = raw.length) {
  loadSingle({ name: "Typo", title: "Typo", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "typo", raw, collapsed: false, children: [] }] });
  startEditing("typo", caret);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Cm6BlockEditor id="typo" surface="main" scope={null} />, root);
  const shown = () => root.querySelector(".cm-content")!.textContent;
  return { root, editor: editorIn(root), shown };
}

it("is on by default and shows the view's glyph while the caret is elsewhere, the ASCII while it touches", () => {
  expect(initial).toBe("render");
  const raw = "a -- b -> c";
  const { editor, shown } = mount(raw);
  expect(shown()).toBe("a – b → c");
  for (const at of [2, 3, 4]) {
    editor.select(at);
    expect(shown()).toBe("a -- b → c");
  }
  editor.select(1, 9);
  expect(shown(), "a selection over a glyph shows what it copies").toBe("a -- b -> c");
  expect(editor.value()).toBe(raw);
  expect(node("typo").raw).toBe(raw);
});

it("keeps code spans raw, as the view does", () => {
  const { editor, shown } = mount("x -- y `c -- d`");
  editor.select(0);
  expect(shown()).toBe("x – y c -- d");
  editor.select(9);
  expect(shown()).toBe("x – y `c -- d`");
});

it("keeps the ASCII under the glyph: typing and Backspace edit the characters the file holds", () => {
  const raw = "a -- b";
  const { editor } = mount(raw);
  editor.select(4);
  editor.keydown({ key: "Backspace" });
  expect(editor.value()).toBe("a - b");
  expect(node("typo").raw).toBe("a - b");
  editor.select(3);
  editor.type("-");
  expect(node("typo").raw).toBe("a -- b");
});

it("shows the ASCII everywhere when the setting is off", () => {
  setTypographyModeSig("off");
  const { shown } = mount("a -- b");
  expect(shown()).toBe("a -- b");
});

it("follows a change of the setting while the editor is open", () => {
  const { shown } = mount("a -- b");
  expect(shown()).toBe("a – b");
  setTypographyModeSig("off");
  expect(shown()).toBe("a -- b");
  setTypographyModeSig("render");
  expect(shown()).toBe("a – b");
});
