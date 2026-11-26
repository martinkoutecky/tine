import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { Decoration, EditorView } from "@codemirror/view";
import { glyphDecorations, glyphWork, previewGlyphs } from "../editor/cm6Preview";
import { GlyphWidget, PreviewWidget } from "../editor/cm6PreviewWidget";
import { render } from "solid-js/web";
import { Cm6BlockEditor } from "./Cm6BlockEditor";
import { initParser } from "../render/parse";
import { node, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { startEditing, endEdit } from "../editorController";
import { editorIn } from "../tests/editorDriver";
import { setTypographyModeSig, typographyMode } from "../ui";
import { closeInPageFind, openInPageFind } from "../inpageFind";

// clicking into a block must not change how `--` or `->` look. With typographic replacements on
// (the default), the editor draws the view's glyphs over the ASCII the file keeps, and shows the ASCII where the caret is.
let dispose: (() => void) | undefined;
const initial = typographyMode();
beforeAll(() => {
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  return initParser();
});
afterEach(() => { vi.restoreAllMocks(); dispose?.(); dispose = undefined; endEdit("page-navigation"); resetStore(); setTypographyModeSig(initial); document.body.innerHTML = ""; });

function mount(raw: string, caret = raw.length) {
  loadSingle({ name: "Typo", title: "Typo", kind: "page", format: "md", pre_block: null,
    blocks: [{ id: "typo", raw, collapsed: false, children: [] }] });
  startEditing("typo", caret);
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <Cm6BlockEditor id="typo" surface="main" scope={null} />, root);
  const shown = () => root.querySelector(".cm-content")!.textContent;
  const view = () => EditorView.findFromDOM(root.querySelector<HTMLElement>(".cm-editor")!)!;
  return { root, editor: editorIn(root), shown, view };
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

it("draws no glyphs while in-page Find is open, as the view does", () => {
  const { shown } = mount("a -- b", 0);
  expect(shown()).toBe("a – b");
  openInPageFind();
  expect(shown()).toBe("a -- b");
  closeInPageFind({ restoreFocus: false });
  expect(shown()).toBe("a – b");
});

it("draws none inside a concealed construct either while Find is open (its widget is the view's renderer)", () => {
  const { shown } = mount("**a -- b** c");
  expect(shown()).toBe("a – b c");
  openInPageFind();
  expect(shown()).toBe("a -- b c");
  closeInPageFind({ restoreFocus: false });
  expect(shown()).toBe("a – b c");
});

// A caret move costs what it changes on screen, not the size of the block: glyph decorations are built only for what is
// in view, and a move that reveals or hides no glyph and changes no concealed construct builds none.
describe("glyph work per caret move", () => {
  const LINES = 2000;
  const big = Array.from({ length: LINES }, (_, i) => i === 1 ? "**x -- y** z" : `line ${i} -- end`).join("\n");
  const at = (line: number, column: number) => big.split("\n").slice(0, line).reduce((n, l) => n + l.length + 1, 0) + column;
  /** Our decorations made from here on (CodeMirror's own gap widgets are its baseline, not counted). */
  const count = () => {
    glyphWork.examined = 0; glyphWork.built = 0;
    const replace = vi.spyOn(Decoration, "replace");
    return () => replace.mock.calls.filter(([spec]) => spec.widget instanceof GlyphWidget || spec.widget instanceof PreviewWidget).length;
  };

  it("(a) a move inside a line that touches no trigger builds no decoration and examines no glyph beyond its own", () => {
    const { editor } = mount(big, 0);
    editor.select(at(5, 1));
    const made = count();
    editor.select(at(5, 2));
    editor.select(at(7, 3));
    expect(made()).toBe(0);
    expect(glyphWork.built).toBe(0);
    expect(glyphWork.examined, "one bisection endpoint per selection, before and after").toBeLessThanOrEqual(4);
  });

  it("(b) entering and leaving a trigger rebuilds only the glyphs in view", () => {
    const { editor, view } = mount(big, 0);
    const inView = previewGlyphs(big, "md").filter(g => view().visibleRanges.some(r => g.from <= r.to && g.to >= r.from)).length;
    expect(inView, "jsdom draws part of the block, not all of it").toBeLessThan(LINES);
    editor.select(at(5, 1));
    count();
    editor.select(at(5, 8));
    expect(glyphWork.built).toBeLessThanOrEqual(inView);
    expect(glyphWork.examined).toBeLessThanOrEqual(inView + 4);
    editor.select(at(5, 1));
    expect(glyphWork.built).toBeLessThanOrEqual(2 * inView);
  });

  it("(c) revealing an enclosing construct draws the glyph inside it", () => {
    const { editor, root } = mount(big, 0);
    editor.select(at(1, 11));
    const line = () => root.querySelectorAll(".cm-line")[1].textContent;
    expect(line(), "the bold widget is the view's renderer").toBe("x – y z");
    editor.select(at(1, 1));
    expect(line()).toBe("**x – y** z");
  });

  it("(d) a viewport examines only the glyphs inside it", () => {
    const glyphs = previewGlyphs(big, "md");
    glyphWork.examined = 0;
    const from = at(1000, 0), to = at(1010, 0);
    const drawn = glyphDecorations(glyphs, [{ from, to }], [], [{ from: 0, to: 0 }], g => g);
    expect(drawn).toHaveLength(10);
    expect(glyphWork.examined).toBeLessThanOrEqual(11);
  });
});
