import { afterEach, beforeEach, expect, it, vi } from "vitest";

// jsdom cannot dispatch a trusted event, so the hold's document listeners are
// recorded at import and called with the event fields they read.
const listeners = vi.hoisted(() => {
  const recorded: [string, EventListener][] = [];
  const add = document.addEventListener.bind(document);
  document.addEventListener = ((type: string, listener: EventListener, options?: boolean | AddEventListenerOptions) => {
    recorded.push([type, listener]);
    add(type, listener, options);
  }) as typeof document.addEventListener;
  return recorded;
});

import { armFeedHold, captureReadingViewport, endFeedHold, feedHoldHandles, installFeedHoldVeto, noteEditorOpening } from "./readingViewportAnchor";
import { registerScrollOwnership } from "../routeScrollAnchor";
import { captureEditorScrollAnchor } from "../editor/scrollAnchor";
import { preserveAboveViewport, trackViewportAnchor } from "../scrollContainer";

/** The hold's listener for `type`: registered in the one batch whose wheel, touchstart and drop share a listener. */
const only = (type: string) => {
  const drop = listeners.findIndex(([t, l]) => t === "drop" && listeners.some(([u, m]) => u === "wheel" && m === l));
  expect(drop).toBeGreaterThan(0);
  const batch = listeners.slice(drop - 4, drop + 3);
  return batch.find(([t]) => t === type)![1];
};
const send = (type: string, fields: Record<string, unknown>) => only(type)(fields as unknown as Event);
const click = (target: Element, fields: Record<string, unknown> = {}) =>
  send("pointerdown", { isTrusted: true, button: 0, pointerType: "mouse", target, ...fields });

let resize: (() => void)[] = [];
let observed: Element[] = [];
beforeEach(() => {
  resize = []; observed = [];
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { resize.push(callback); }
    observe(el: Element) { observed.push(el); }
    unobserve() {}
    disconnect() { observed = []; }
  });
});
afterEach(() => { send("wheel", {}); endFeedHold(); installFeedHoldVeto(() => false); document.body.innerHTML = ""; vi.restoreAllMocks(); vi.unstubAllGlobals(); });

/** A Journals feed: content `above` the editor (CSS px), a 500 px pane, the browser's range clamp, and optionally
 * WebKit's whole-pixel `scrollTop`. Every assignment is logged. */
function feed({ above = 520, below = 1000, scroll = 300, truncate = false } = {}) {
  const scroller = document.createElement("main"); scroller.className = "main-content";
  const page = document.createElement("div"); page.className = "page";
  const earlier = document.createElement("div"); earlier.className = "journal-feed-day";
  const day = document.createElement("div"); day.className = "journal-feed-day";
  const body = document.createElement("div");
  const block = document.createElement("div"); block.dataset.blockId = "held";
  const editor = document.createElement("textarea");
  block.append(editor); body.append(block); day.append(body); page.append(earlier, day); scroller.append(page); document.body.append(scroller);
  const geometry = { above, below, width: 800, height: 500 };
  const max = () => Math.max(0, geometry.above + geometry.below - geometry.height);
  let value = scroll;
  const writes: number[] = [];
  Object.defineProperty(scroller, "scrollTop", { configurable: true,
    get: () => Math.min(value, max()),
    set: (v: number) => { writes.push(v); value = Math.min(Math.max(truncate ? Math.trunc(v) : v, 0), max()); } });
  Object.defineProperty(scroller, "scrollHeight", { configurable: true, get: () => geometry.above + geometry.below });
  Object.defineProperty(scroller, "clientHeight", { configurable: true, get: () => geometry.height });
  Object.defineProperty(scroller, "clientWidth", { configurable: true, get: () => geometry.width });
  vi.spyOn(scroller, "getBoundingClientRect").mockImplementation(() => ({ top: 0, left: 0, width: geometry.width, bottom: geometry.height, height: geometry.height } as DOMRect));
  const top = () => geometry.above - scroller.scrollTop;
  const editorRect = vi.spyOn(editor, "getBoundingClientRect").mockImplementation(() => ({ top: top(), bottom: top() + 30 } as DOMRect));
  registerScrollOwnership(scroller, () => true);
  const open = (fits: (shift: number) => boolean = () => true) => {
    click(editor);
    editor.focus();
    noteEditorOpening(editor);
    armFeedHold(editor, scroller, fits);
  };
  const deliver = () => { for (const callback of resize) callback(); };
  return { scroller, page, earlier, day, body, block, editor, geometry, writes, top, open, deliver, editorRect };
}

it("holds a click-opened editor still through growth above, observing only the day list and its own day body", () => {
  const f = feed();
  f.open();
  expect(observed).toEqual([f.page, f.body]);
  expect(f.top()).toBe(220);
  f.geometry.above += 32;
  f.deliver();
  expect(f.top()).toBe(220);
  expect(f.writes).toEqual([332]);
  f.deliver(); // repeat delivery, same layout: no write
  expect(f.writes).toEqual([332]);
});

it("never arms for a keyboard, touch, untrusted, secondary or programmatic opening, beside Find, or before the route owns its scroll", () => {
  const cases: [string, (f: ReturnType<typeof feed>) => void][] = [
    ["keyboard", f => { f.editor.focus(); noteEditorOpening(f.editor); }],
    ["touch", f => { click(f.editor, { pointerType: "touch" }); f.editor.focus(); noteEditorOpening(f.editor); }],
    ["untrusted", f => { click(f.editor, { isTrusted: false }); f.editor.focus(); noteEditorOpening(f.editor); }],
    ["secondary", f => { click(f.editor, { button: 2 }); f.editor.focus(); noteEditorOpening(f.editor); }],
    ["key between click and settle", f => { click(f.editor); f.editor.focus(); noteEditorOpening(f.editor); send("keydown", { key: "a", target: f.editor }); }],
    ["another block's editor", f => { const other = document.createElement("div"); other.dataset.blockId = "other"; f.body.append(other); click(other); f.editor.focus(); noteEditorOpening(f.editor); }],
    ["outside the feed", f => { const outside = document.createElement("div"); outside.dataset.blockId = "held"; document.body.append(outside); click(outside); f.editor.focus(); noteEditorOpening(f.editor); }],
    ["Find open", f => { installFeedHoldVeto(() => true); click(f.editor); f.editor.focus(); noteEditorOpening(f.editor); }],
    ["route pursuing", f => { registerScrollOwnership(f.scroller, () => false); click(f.editor); f.editor.focus(); noteEditorOpening(f.editor); }],
  ];
  for (const [name, setup] of cases) {
    send("wheel", {});
    const f = feed();
    setup(f);
    armFeedHold(f.editor, f.scroller, () => true);
    f.geometry.above += 32;
    f.deliver();
    expect(feedHoldHandles(f.scroller), name).toBe(false);
    expect(f.writes, name).toEqual([]);
    installFeedHoldVeto(() => false);
    document.body.innerHTML = "";
  }
});

it("ends on the first interaction other than text typed into the editor, and never re-arms on its own", () => {
  const ends: [string, (f: ReturnType<typeof feed>) => void][] = [
    ["pointerdown", f => click(f.editor)],
    ["wheel", () => send("wheel", {})],
    ["touchstart", () => send("touchstart", {})],
    ["drop", () => send("drop", {})],
    ["Ctrl+Z", f => send("keydown", { key: "z", ctrlKey: true, target: f.editor })],
    ["Enter", f => send("keydown", { key: "Enter", target: f.editor })],
    ["Tab", f => send("keydown", { key: "Tab", shiftKey: true, target: f.editor })],
    ["composition", f => send("keydown", { key: "a", isComposing: true, target: f.editor })],
    ["text elsewhere", f => send("keydown", { key: "a", target: f.scroller })],
    ["Find opened", () => endFeedHold()],
  ];
  for (const [name, end] of ends) {
    const f = feed();
    f.open();
    send("keydown", { key: "a", target: f.editor });
    send("keydown", { key: "Backspace", target: f.editor });
    expect(feedHoldHandles(f.scroller), name).toBe(true);
    end(f);
    // The click's own frame, a focus return and a later watch settle do not re-arm.
    f.editor.focus(); armFeedHold(f.editor, f.scroller, () => true);
    f.geometry.above += 32;
    f.deliver();
    expect(f.writes, name).toEqual([]);
    expect(feedHoldHandles(f.scroller), name).toBe(false);
    document.body.innerHTML = "";
  }
});

it("check: a writer that kept the editor is accepted; one that moved it ends the hold with no write", () => {
  const f = feed();
  f.open();
  f.geometry.above += 32;
  f.scroller.scrollTop = 332; // native anchoring on the focused editable
  f.writes.length = 0;
  f.deliver();
  expect(f.writes).toEqual([]);
  expect(feedHoldHandles(f.scroller)).toBe(true);
  f.scroller.scrollTop = 400; // a foreign write moved the editor
  f.writes.length = 0;
  f.deliver();
  expect(f.writes).toEqual([]);
  expect(feedHoldHandles(f.scroller)).toBe(false);
});

it("check: the pinned clamp probe keeps the maximum of a kept write and compensates after the range clamps", () => {
  const f = feed({ above: 580, below: 300, scroll: 360 }); // (s, max) = (360, 380), editor at 220
  f.open();
  f.geometry.above = 680; // (460, 480) after the kept write
  f.deliver();
  expect(f.writes).toEqual([460]);
  f.geometry.above = 620; f.geometry.below = 300; // the range shrinks to 420 and clamps; the editor is 20 above
  expect(f.scroller.scrollTop).toBe(420);
  f.deliver();
  expect(f.writes).toEqual([460, 400]);
  expect(f.top()).toBe(220);
  expect(feedHoldHandles(f.scroller)).toBe(true);
});

it("check: WebKit's whole-pixel offset within 1 px is held, and the same layout is not written again", () => {
  const f = feed({ truncate: true });
  f.open();
  f.geometry.above += 32.75;
  f.deliver();
  expect(f.writes).toEqual([332.75]);
  expect(f.scroller.scrollTop).toBe(332);
  expect(Math.abs(f.top() - 220)).toBeLessThanOrEqual(1);
  f.deliver();
  expect(f.writes).toEqual([332.75]);
  expect(feedHoldHandles(f.scroller)).toBe(true);
});

it("check: an unreachable compensation writes the nearest offset and ends the hold", () => {
  const f = feed();
  f.open();
  f.geometry.above -= 400; // a day above removed: 300 - 400 is below 0
  f.deliver();
  expect(f.writes).toEqual([0]);
  expect(feedHoldHandles(f.scroller)).toBe(false);
});

it("check: a pane resize or a compensation that would hide the caret ends the hold with no write", () => {
  const f = feed();
  f.open();
  f.geometry.height = 410;
  f.geometry.above -= 64;
  f.deliver();
  expect(f.writes).toEqual([]);
  expect(feedHoldHandles(f.scroller)).toBe(false);
  document.body.innerHTML = "";
  const g = feed();
  const shifts: number[] = [];
  g.open(shift => { shifts.push(shift); return false; });
  g.geometry.above += 32;
  g.deliver();
  expect(shifts).toEqual([32]);
  expect(g.writes).toEqual([]);
  expect(feedHoldHandles(g.scroller)).toBe(false);
});

it("ends when its editor detaches without a blur, and on the editor's teardown with both boxes unchanged", () => {
  const f = feed();
  f.open();
  f.block.remove();
  f.deliver();
  expect(feedHoldHandles(f.scroller)).toBe(false);
  document.body.innerHTML = "";
  const g = feed();
  g.open();
  endFeedHold(document.createElement("textarea")); // another editor's teardown
  expect(observed).toHaveLength(2);
  endFeedHold(g.editor);
  expect(observed).toEqual([]);
  expect(feedHoldHandles(g.scroller)).toBe(false);
  g.open(); // the next click-opened editor in the same pane holds again
  expect(feedHoldHandles(g.scroller)).toBe(true);
});

it("every restore on the held scroller hands over, whatever it captured; after the end each is its own again", () => {
  const f = feed();
  vi.stubGlobal("MutationObserver", class { constructor(public callback: () => void) { mutations.push(callback); } observe() {} disconnect() {} });
  const mutations: (() => void)[] = [];
  const group = document.createElement("div"); group.className = "reference-group"; f.earlier.append(group);
  let groupTop = 10;
  vi.spyOn(group, "getBoundingClientRect").mockImplementation(() => ({ top: groupTop - f.scroller.scrollTop, bottom: groupTop - f.scroller.scrollTop + 40 } as DOMRect));
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => group });
  try {
    f.scroller.style.overflowY = "auto";
    const stop = trackViewportAnchor(group); // M4: a snapshot taken before arming
    f.editor.focus();
    const commit = captureEditorScrollAnchor(f.editor, f.scroller)!; // M2
    f.open();
    const reading = captureReadingViewport(f.page, f.scroller)!; // M1, anchored on the focused editor
    // +32 above the group and +32 between it and the editor.
    groupTop += 32; f.geometry.above += 64;
    mutations[0]();
    expect(f.writes).toEqual([364]);
    expect(f.top()).toBe(220);
    commit.restore();
    reading.restore();
    expect(f.writes).toEqual([364]);
    // M5: a box wholly above the view.
    const box = document.createElement("div"); f.earlier.append(box);
    vi.spyOn(box, "getBoundingClientRect").mockImplementation(() => ({ top: -200, bottom: -100 } as DOMRect));
    preserveAboveViewport(box, () => { f.geometry.above += 16; }, f.scroller);
    expect(f.writes).toEqual([364, 380]);
    expect(f.top()).toBe(220);
    reading.restore(); // delegated again: rebased to the shown layout
    expect(f.writes).toEqual([364, 380]);
    // After the end: M1 holds what is shown now, from its last rebase.
    endFeedHold();
    f.geometry.above += 10;
    reading.restore();
    expect(f.writes).toEqual([364, 380, 390]);
    reading.release(); stop();
  } finally { Reflect.deleteProperty(document, "elementFromPoint"); }
});
