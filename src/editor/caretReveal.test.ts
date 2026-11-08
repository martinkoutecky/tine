// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// jsdom has no layout: the mirror measurement is replaced by a fixed caret row so the reveal RULE is
// what is tested (GH #745): no move while the caret row is visible, the least move otherwise, below pins.
let row: { top: number; bottom: number } | null = { top: 0, bottom: 20 };
vi.mock("./caretRows", () => ({ textareaCaretRow: () => row }));
import { caretViewRow, editorCaretRow, keyboardCaretOffset, registerCaretRowSource, revealCaret, revealCaretAboveKeyboard, revealCaretSettled } from "./caretReveal";
import { applyAndroidInterfaceZoom } from "../zoom";

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
it("keeps the row within the visual viewport and above a dock without centring it (GH #783)", () => {
  const above = fixture(20); // y=120, below the scroller top but above a panned viewport
  revealCaret(above.editor, above.scroller, { top: 150, bottom: 450 });
  expect(above.scroller.scrollTop).toBe(962); // row top becomes 158
  const below = fixture(330); // y=430..450, above the scroller bottom but under the dock margin
  revealCaret(below.editor, below.scroller, { top: 100, bottom: 450 });
  expect(below.scroller.scrollTop).toBe(1008); // row bottom becomes 442
  expect(revealCaret(below.editor, below.scroller, { top: 100, bottom: 450 })).toBe(false);
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
it("keeps the caret below a page-embed pin while its tall embedded body is still on screen (GH #898)", () => {
  const f = fixture(10, 1);
  const main = f.scroller.querySelector<HTMLElement>(".block-main")!;
  main.getBoundingClientRect = () => ({ bottom: 900 } as DOMRect);
  const controls = document.createElement("div"); controls.setAttribute("data-sticky-row", "");
  controls.getBoundingClientRect = () => ({ bottom: 60 } as DOMRect);
  main.append(controls, f.editor.closest(".ls-block")!);
  f.editor.focus();
  revealCaret(f.editor, f.scroller);
  expect(f.scroller.scrollTop).toBe(1000 + 10 - 30 - 8);
});
it("keeps a revealed caret below a wrapped sticky seen notice", () => {
  const f = fixture(30);
  f.scroller.style.scrollPaddingTop = "82px";
  revealCaret(f.editor, f.scroller);
  expect(f.scroller.scrollTop).toBe(1000 + 30 - 82 - 8);
});
it("reserves both the seen notice and nested pinned ancestors", () => {
  const f = fixture(30, 2);
  f.scroller.style.scrollPaddingTop = "82px";
  f.scroller.style.setProperty("--seen-header-height", "82px");
  revealCaret(f.editor, f.scroller);
  expect(f.scroller.scrollTop).toBe(1000 + 30 - 82 - 60 - 8);
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

it("GH #745: an image that loads after the frame cap re-reveals a caret it pushed out of view", () => {
  const f = fixture(200);
  revealCaretSettled(f.editor, f.scroller, 2, 3);
  for (let i = 0; i < 3; i++) { f.setTop(200 + i); step(1); } // layout never settles: the cap fires
  expect(f.scroller.scrollTop).toBe(1000); // visible, so the cap reveal did not move it
  expect(frames.length).toBe(0);
  f.setTop(700); // the late image lands above the editor and pushes the caret below the view
  const img = document.createElement("img"); f.scroller.append(img);
  img.dispatchEvent(new Event("load")); // does not bubble: only a capturing listener sees it
  step(1);
  expect(f.scroller.scrollTop).toBe(1000 + (700 + 20 - (500 - 8)));
});
it("GH #745: the late watch ends on user input, blur and cancel", () => {
  for (const end of [(f: ReturnType<typeof fixture>) => f.scroller.dispatchEvent(new Event("wheel")), (f: ReturnType<typeof fixture>) => f.editor.blur()]) {
    const f = fixture(200);
    revealCaretSettled(f.editor, f.scroller);
    step(2);
    end(f);
    f.setTop(700);
    f.scroller.append(document.createElement("img"));
    f.scroller.lastElementChild!.dispatchEvent(new Event("load"));
    step(2);
    expect(f.scroller.scrollTop).toBe(1000);
  }
  const g = fixture(200);
  const cancel = revealCaretSettled(g.editor, g.scroller);
  step(2); cancel(); g.setTop(700);
  const img = document.createElement("img"); g.scroller.append(img); img.dispatchEvent(new Event("load"));
  step(2);
  expect(g.scroller.scrollTop).toBe(1000);
});
it("GH #745: the late watch expires", () => {
  vi.useFakeTimers();
  try {
    const f = fixture(200);
    revealCaretSettled(f.editor, f.scroller, 2, 40, 500);
    step(2);
    vi.advanceTimersByTime(600);
    f.setTop(700);
    const img = document.createElement("img"); f.scroller.append(img); img.dispatchEvent(new Event("load"));
    step(2);
    expect(f.scroller.scrollTop).toBe(1000);
  } finally { vi.useRealTimers(); }
});

// GH #783: the keyboard toolbar reveal must not carry the caret row off the top of a tall editor.
import { scrollToClearToolbar } from "./caretReveal";
/** A 500px scroller at viewport y=100 (pins none: the caret may sit down to y=108), an editor of `height` at `top`. */
function toolbarFixture(top: number, height: number) {
  const scroller = document.createElement("div");
  const block = document.createElement("div"); block.className = "ls-block";
  const editor = document.createElement("textarea");
  block.append(editor); scroller.append(block); document.body.append(scroller);
  editor.focus();
  Object.defineProperty(scroller, "clientHeight", { value: 500 });
  scroller.getBoundingClientRect = () => ({ top: 100 } as DOMRect);
  editor.getBoundingClientRect = () => ({ top, bottom: top + height, height } as DOMRect);
  return { scroller, editor };
}
const TOOLBAR_TOP = 600; // caret bottom target 540 with 20px body lines
it("leaves three body lines below the caret in a short editor (GH #783)", () => {
  const f = toolbarFixture(560, 100);
  expect(scrollToClearToolbar(f.editor, f.scroller, TOOLBAR_TOP)).toBe(580 - 540);
});
it("does not scroll when the editor already clears the toolbar", () => {
  const f = toolbarFixture(400, 100);
  expect(scrollToClearToolbar(f.editor, f.scroller, TOOLBAR_TOP)).toBe(0);
});
it("GH #783: a tall editor with the caret on its first line is not scrolled so that line leaves the top", () => {
  row = { top: 0, bottom: 20 };
  const f = toolbarFixture(400, 800); // bottom 1200; the editor cannot fit the 484px room
  // the old rule scrolled 608px, taking the caret row (y 400) to y -208, above the scroller top (100)
  expect(scrollToClearToolbar(f.editor, f.scroller, TOOLBAR_TOP)).toBe(0);
});
it("GH #783: a tall editor places its caret three body lines above the toolbar", () => {
  row = { top: 300, bottom: 320 }; // caret row at y 700..720 in the viewport
  const f = toolbarFixture(400, 800);
  expect(scrollToClearToolbar(f.editor, f.scroller, TOOLBAR_TOP)).toBe(720 - 540);
});
it("without a caret measurement it leaves the scroll alone", () => {
  row = null;
  const f = toolbarFixture(400, 800);
  expect(scrollToClearToolbar(f.editor, f.scroller, TOOLBAR_TOP)).toBe(0);
});

// GH #783, comment 14: reserve three body lines below the caret, including when recovering a line under pins.
function sized(f: ReturnType<typeof fixture>, editorTop: number, height: number) {
  f.editor.getBoundingClientRect = () => {
    const top = 100 + editorTop - (f.scroller.scrollTop - 1000);
    return { top, bottom: top + height, height } as DOMRect;
  };
  return f;
}
const KEYBOARD = { top: 0, bottom: 450 };
it("GH #783: a line under the pinned parents goes back three body lines above the keyboard toolbar", () => {
  const f = sized(fixture(10, 2), 10, 20); // y 110..130, under two pins (100..160)
  expect(revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD)).toBe(true);
  expect(f.editor.getBoundingClientRect().bottom).toBe(390);
  expect(f.scroller.scrollTop).toBe(1000 - 260);
  expect(revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD)).toBe(false); // settled: stays put
});
it("GH #783: a visible line is left where it is, as 0.7.4 left it", () => {
  const f = sized(fixture(200, 2), 200, 20);
  expect(keyboardCaretOffset(f.editor, f.scroller, KEYBOARD)).toBe(0);
  expect(revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD)).toBe(false);
  expect(f.scroller.scrollTop).toBe(1000);
});
it("GH #783: a multi-line editor is placed by its caret, so typing does not move it again", () => {
  row = { top: 0, bottom: 20 }; // caret on the first of three lines
  const f = sized(fixture(-100), -100, 60);
  revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD);
  expect(caretViewRow(f.editor, f.scroller)!.bottom).toBe(390);
  expect(scrollToClearToolbar(f.editor, f.scroller, KEYBOARD.bottom)).toBe(0);
});
it("GH #783: in an editor taller than the room the caret row goes three lines above the toolbar", () => {
  row = { top: 400, bottom: 420 };
  const f = sized(fixture(-500), -500, 900); // caret row y -0..20, above the scroller top
  revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD);
  expect(f.editor.getBoundingClientRect().top + 420).toBe(390);
});
it("GH #783: a line below the toolbar moves to the three-line band", () => {
  const f = sized(fixture(400), 400, 20); // y 500..520
  revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD);
  expect(f.editor.getBoundingClientRect().bottom).toBe(390);
});

it.each([20, 40])("GH #783: a %ipx parent leaves its first child row visible below the caret", height => {
  const f = sized(fixture(320), 320, height); // caret y 420..440: visible but too low
  const child = document.createElement("div");
  child.textContent = "First child";
  f.editor.parentElement!.append(child);
  child.getBoundingClientRect = () => ({ top: f.editor.getBoundingClientRect().bottom + 4, bottom: f.editor.getBoundingClientRect().bottom + 24 } as DOMRect);
  expect(revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD)).toBe(true);
  expect(caretViewRow(f.editor, f.scroller)!.bottom).toBe(390);
  expect(child.getBoundingClientRect().bottom).toBeLessThan(KEYBOARD.bottom);
  expect(revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD)).toBe(false);
});

it("uses body typography for the three-line clearance even in a heading editor", () => {
  row = { top: 0, bottom: 40 };
  const f = sized(fixture(320), 320, 44);
  f.editor.style.setProperty("--ls-block-line-height", "26px");
  revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD);
  expect(caretViewRow(f.editor, f.scroller)!.bottom).toBe(450 - 3 * 26);
});

it("reduces the three-line gap when pins leave room only for the caret", () => {
  const f = sized(fixture(400, 2), 400, 800);
  revealCaretAboveKeyboard(f.editor, f.scroller, { top: 0, bottom: 220 });
  expect(caretViewRow(f.editor, f.scroller)!.top).toBe(168); // pins + margin, never above the top
});

it("accepts the browser's end-of-page clamp without repeated scrolling", () => {
  const f = sized(fixture(320), 320, 20);
  let scroll = 1000;
  Object.defineProperty(f.scroller, "scrollTop", { get: () => scroll, set: n => { scroll = Math.max(0, Math.min(1020, n)); } });
  revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD);
  expect(scroll).toBe(1020); // desired +50, only +20 available
  expect(caretViewRow(f.editor, f.scroller)!.bottom).toBe(420); // still visible; no artificial blank tail
  revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD);
  expect(scroll).toBe(1020);
});

// GH #783 follow-up: Android's interface zoom is CSS zoom on the root. Rects are viewport pixels, but the pin height,
// margins, mirror rows and scrollTop are CSS pixels. Unscaled, at zoom 1.25 the visible top ended 7px above the
// painted pins, a line under them counted as visible, and corrections moved by the wrong amount.
describe.each([1.25, 1.5])("at interface zoom %d", z => {
  beforeEach(() => applyAndroidInterfaceZoom(z));
  afterEach(() => applyAndroidInterfaceZoom(1));
  /** Two pinned parents painted 100..100+60z; the edited line `lineTop` px below the scroller top, 20 CSS px tall;
   * scrolling by d CSS px moves it by d*z viewport px. */
  function zoomed(lineTop: number) {
    const f = fixture(lineTop, 2);
    f.editor.getBoundingClientRect = () => {
      const top = 100 + lineTop - (f.scroller.scrollTop - 1000) * z;
      return { top, bottom: top + 20 * z, height: 20 * z } as DOMRect;
    };
    return f;
  }
  const pinsBottom = 100 + 60 * z;
  it("a line just under the painted pins is not visible: the reveal brings it below them", () => {
    const f = zoomed(60 * z - 2); // starts 2px inside the second pin, below the unscaled 60+8
    expect(revealCaret(f.editor, f.scroller)).toBe(true);
    expect(f.editor.getBoundingClientRect().top).toBeCloseTo(pinsBottom + 8 * z, 6);
  });
  it("the visible top is the painted pins' bottom plus the margin", () => {
    const f = zoomed(200);
    const g = caretViewRow(f.editor, f.scroller)!;
    expect(g.visibleTop).toBeCloseTo(pinsBottom + 8 * z, 6);
    expect(g.visibleBottom).toBeCloseTo(100 + (500 - 8) * z, 6);
  });
  it("the keyboard placement leaves three zoomed body lines and a visible child below the caret", () => {
    const f = zoomed(60 * z - 2);
    expect(revealCaretAboveKeyboard(f.editor, f.scroller, KEYBOARD)).toBe(true);
    expect(f.editor.getBoundingClientRect().bottom).toBeCloseTo(450 - 3 * 20 * z, 6);
    expect(f.editor.getBoundingClientRect().bottom + 24 * z).toBeLessThan(450);
  });
});

// GH #783 follow-up: the CM6 editor (a div) supplies its measured caret row; the keyboard-time rules use it like the textarea's.
describe("a non-textarea editor with a registered caret row", () => {
  function cm6Fixture(rowTop: number, { height = 800, pins = 0, z = 1, rowHeight = 20 } = {}) {
    const f = fixture(0, pins);
    const div = document.createElement("div");
    div.tabIndex = 0;
    f.editor.replaceWith(div);
    div.focus();
    div.getBoundingClientRect = () => ({ top: 100 + rowTop - (f.scroller.scrollTop - 1000) * z, bottom: 100 + rowTop + height * z - (f.scroller.scrollTop - 1000) * z, height: height * z } as DOMRect);
    const source = () => { const y = div.getBoundingClientRect().top; return { top: y, bottom: y + rowHeight * z }; };
    return { ...f, div, source };
  }
  it("has no row without a source, and none once it is unregistered", () => {
    const f = cm6Fixture(0);
    expect(editorCaretRow(f.div, f.scroller)).toBeNull();
    const off = registerCaretRowSource(f.div, f.source);
    expect(editorCaretRow(f.div, f.scroller)).not.toBeNull();
    off();
    expect(editorCaretRow(f.div, f.scroller)).toBeNull();
  });
  it("puts a caret line that left the view three body lines above the keyboard", () => {
    const f = cm6Fixture(-300); // caret row at viewport y -200
    registerCaretRowSource(f.div, f.source);
    const band = { top: 0, bottom: 480 };
    // The distant editor bottom does not matter: the caret row goes to 480 - 3 * 20.
    expect(keyboardCaretOffset(f.div, f.scroller, band)).toBe(-180 - (480 - 3 * 20));
    expect(revealCaretAboveKeyboard(f.div, f.scroller, band)).toBe(true);
    expect(f.source().bottom).toBe(480 - 3 * 20);
    expect(revealCaretAboveKeyboard(f.div, f.scroller, band)).toBe(false); // placed: nothing further
  });
  it("lets the toolbar rule clear the caret row of a tall editor, not its bottom (never upward)", () => {
    const above = cm6Fixture(100); // caret row at y 200..220, the toolbar at 480: the row is clear
    registerCaretRowSource(above.div, above.source);
    expect(scrollToClearToolbar(above.div, above.scroller, 480)).toBe(0);
    const below = cm6Fixture(450); // caret row at y 550..570, under the toolbar
    registerCaretRowSource(below.div, below.source);
    expect(scrollToClearToolbar(below.div, below.scroller, 480)).toBe(570 - (480 - 3 * 20));
  });

  it.each([20, 40])("a %ipx CM6 parent leaves its first child visible", height => {
    const f = cm6Fixture(320, { height });
    registerCaretRowSource(f.div, f.source);
    const child = document.createElement("div");
    f.div.after(child);
    child.getBoundingClientRect = () => ({ top: f.div.getBoundingClientRect().bottom + 4, bottom: f.div.getBoundingClientRect().bottom + 24 } as DOMRect);
    expect(revealCaretAboveKeyboard(f.div, f.scroller, KEYBOARD)).toBe(true);
    expect(f.source().bottom).toBe(KEYBOARD.bottom - 3 * 20);
    expect(child.getBoundingClientRect().bottom).toBeLessThan(KEYBOARD.bottom);
    expect(scrollToClearToolbar(f.div, f.scroller, KEYBOARD.bottom)).toBe(0);
    expect(revealCaretAboveKeyboard(f.div, f.scroller, KEYBOARD)).toBe(false);
  });

  it("a tall CM6 block with its caret above the three-line band stays put", () => {
    const f = cm6Fixture(120, { pins: 2 });
    registerCaretRowSource(f.div, f.source);
    expect(revealCaretAboveKeyboard(f.div, f.scroller, KEYBOARD)).toBe(false);
    expect(scrollToClearToolbar(f.div, f.scroller, KEYBOARD.bottom)).toBe(0);
    expect(f.scroller.scrollTop).toBe(1000);
  });

  it.each([1, 1.25, 1.5])("uses body typography, independently of CM6 heading height, at zoom %d", z => {
    applyAndroidInterfaceZoom(z);
    try {
      const f = cm6Fixture(320, { z, rowHeight: 40 });
      // Resolved value on contentDOM; the browser fixture also checks CSS inheritance.
      f.div.style.setProperty("--ls-block-line-height", "26px");
      registerCaretRowSource(f.div, f.source);
      revealCaretAboveKeyboard(f.div, f.scroller, KEYBOARD);
      expect(f.source().bottom).toBeCloseTo(KEYBOARD.bottom - 3 * 26 * z);
    } finally { applyAndroidInterfaceZoom(1); }
  });

  it("reduces the CM6 gap when pins leave room only for the caret", () => {
    const f = cm6Fixture(400, { pins: 2 });
    registerCaretRowSource(f.div, f.source);
    revealCaretAboveKeyboard(f.div, f.scroller, { top: 0, bottom: 220 });
    expect(f.source().top).toBe(100 + 2 * 30 + 8);
  });

  it("accepts the page-end scroll clamp for CM6 without moving the caret again", () => {
    const f = cm6Fixture(320, { height: 20 });
    registerCaretRowSource(f.div, f.source);
    let scroll = 1000;
    Object.defineProperty(f.scroller, "scrollTop", { get: () => scroll, set: n => { scroll = Math.max(0, Math.min(1020, n)); } });
    revealCaretAboveKeyboard(f.div, f.scroller, KEYBOARD);
    expect(scroll).toBe(1020);
    expect(f.source().bottom).toBe(420); // visible; browser cannot supply all three lines
    revealCaretAboveKeyboard(f.div, f.scroller, KEYBOARD);
    expect(scroll).toBe(1020);
  });
});
