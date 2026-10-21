// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";

// jsdom has no layout: the mirror measurement is replaced by a fixed caret row so the reveal RULE is
// what is tested (GH #745): no move while the caret row is visible, the least move otherwise, below pins.
let row: { top: number; bottom: number } | null = { top: 0, bottom: 20 };
vi.mock("./caretRows", () => ({ textareaCaretRow: () => row }));
import { revealCaret, revealCaretSettled } from "./caretReveal";

const frames: FrameRequestCallback[] = [];
beforeEach(() => {
  row = { top: 0, bottom: 20 };
  frames.length = 0;
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => { frames.push(cb); return frames.length; });
  vi.stubGlobal("cancelAnimationFrame", () => {});
  window.requestAnimationFrame = (cb: FrameRequestCallback) => { frames.push(cb); return frames.length; };
});
afterEach(() => { document.body.innerHTML = ""; vi.unstubAllGlobals(); });

/** A 500px-tall scroller at viewport y=100, an editor whose top is `editorTop` px below the scroller top. */
function fixture(editorTop: number, pins = 0, pinsShown = true) {
  const scroller = document.createElement("div");
  let host: HTMLElement = scroller;
  for (let i = 0; i < pins; i++) {
    const p = document.createElement("div"); p.setAttribute("data-sticky-parent", "");
    const row = document.createElement("div"); row.className = "block-main";
    // A pinned ancestor's own row is scrolled out above the view (top 100); an unpinned one is on screen.
    row.getBoundingClientRect = () => ({ bottom: pinsShown ? 60 : 400 } as DOMRect);
    p.append(row); host.append(p); host = p;
  }
  const block = document.createElement("div"); block.className = "ls-block";
  const editor = document.createElement("textarea");
  block.append(editor); host.append(block); document.body.append(scroller);
  editor.focus();
  Object.defineProperty(scroller, "clientHeight", { value: 500 });
  scroller.scrollTop = 1000;
  let top = editorTop;
  scroller.getBoundingClientRect = () => ({ top: 100 } as DOMRect);
  editor.getBoundingClientRect = () => ({ top: 100 + top - (scroller.scrollTop - 1000) } as DOMRect);
  return { scroller, editor, setTop: (n: number) => { top = n; } };
}

it("does not move the view while the caret row is visible", () => {
  const f = fixture(200);
  expect(revealCaret(f.editor, f.scroller)).toBe(false);
  expect(f.scroller.scrollTop).toBe(1000);
});
it.each([[-477, -477 - 8], [-6, -6 - 8]])("scrolls up by the least amount when the caret row is above (top %i)", (editorTop, delta) => {
  const f = fixture(editorTop);
  expect(revealCaret(f.editor, f.scroller)).toBe(true);
  expect(f.scroller.scrollTop).toBe(1000 + delta);
});
it("scrolls down by the least amount when the caret row is below", () => {
  row = { top: 0, bottom: 20 };
  const f = fixture(560);
  revealCaret(f.editor, f.scroller);
  expect(f.scroller.scrollTop).toBe(1000 + (560 + 20 - (500 - 8)));
});
it("keeps the caret row below the sticky parent pins", () => {
  const f = fixture(10, 2); // two pinned ancestors cover the top 60px
  revealCaret(f.editor, f.scroller);
  expect(f.scroller.scrollTop).toBe(1000 + (10 - (60 + 8)));
});
it("ignores pins whose ancestor row is still on screen", () => {
  const f = fixture(10, 2, false);
  expect(revealCaret(f.editor, f.scroller)).toBe(false); // caret row at 10 is inside 8..492
  expect(f.scroller.scrollTop).toBe(1000);
});
it("measures the caret row inside the editor, not its top", () => {
  row = { top: 400, bottom: 420 };
  const f = fixture(-300); // editor top is above the view but the caret row (y 100) is visible
  expect(revealCaret(f.editor, f.scroller)).toBe(false);
});
it("does nothing without layout, without focus, or for a foreign scroller", () => {
  row = null;
  const f = fixture(-477);
  expect(revealCaret(f.editor, f.scroller)).toBe(false);
  row = { top: 0, bottom: 20 };
  f.editor.blur();
  expect(revealCaret(f.editor, f.scroller)).toBe(false);
  f.editor.focus();
  expect(revealCaret(f.editor, document.createElement("div"))).toBe(false);
  expect(f.scroller.scrollTop).toBe(1000);
});
/** Run queued animation frames one at a time. */
const step = (n = 1) => { for (let i = 0; i < n; i++) frames.shift()?.(0); };

it("reveals once, after the layout has stopped moving, and not at mount", () => {
  const f = fixture(-477);
  revealCaretSettled(f.editor, f.scroller);
  expect(f.scroller.scrollTop).toBe(1000); // nothing synchronously
  step(2); // signature unchanged for 2 frames
  expect(f.scroller.scrollTop).toBe(1000 - 477 - 8);
  expect(frames.length).toBe(0);
});
it("waits while the layout is still moving and reveals the settled position, not the interim one", () => {
  const f = fixture(-6000); // interim layout: editor far above the view
  revealCaretSettled(f.editor, f.scroller);
  step(1);
  f.setTop(200); // the windows settle: the caret is visible after all
  step(1);
  expect(f.scroller.scrollTop).toBe(1000);
  step(3);
  expect(f.scroller.scrollTop).toBe(1000); // visible and stable: never moved
  expect(frames.length).toBe(0);
});
it("reveals after the frame cap even if the layout never settles", () => {
  const f = fixture(-100);
  revealCaretSettled(f.editor, f.scroller, 2, 5);
  for (let i = 0; i < 5; i++) { f.setTop(-100 - i); step(1); }
  expect(f.scroller.scrollTop).not.toBe(1000);
  expect(frames.length).toBe(0);
});
it.each(["wheel", "pointerdown", "touchstart", "keydown"])("yields to a %s from the user", (type) => {
  const f = fixture(-100);
  revealCaretSettled(f.editor, f.scroller);
  f.scroller.dispatchEvent(new Event(type));
  step(4);
  expect(f.scroller.scrollTop).toBe(1000);
});
it("stops when the editor loses focus and when cancelled", () => {
  const f = fixture(-100);
  revealCaretSettled(f.editor, f.scroller);
  f.editor.blur();
  step(4);
  expect(f.scroller.scrollTop).toBe(1000);
  const g = fixture(-100);
  const cancel = revealCaretSettled(g.editor, g.scroller);
  cancel();
  step(4);
  expect(g.scroller.scrollTop).toBe(1000);
});
