// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { cm6CaretReveal } from "../editor/cm6CaretReveal";
import { applyAndroidInterfaceZoom } from "../zoom";

// GH #783 follow-up (Android, CM6 editor). The textarea editor follows a keyboard that opens, or a WebView focus scroll
// that finishes, after the initial settle (MobileKeyboardToolbar -> watchCaretReveal with scroll following) and puts the
// caret three body lines above the keyboard toolbar. The CM6 editor is a contenteditable div: the toolbar only did this for a
// textarea, so a late scroll left the CM6 caret under the keyboard. jsdom has no layout: the caret rect and the
// element rects are given; the rules under test are which events the follow listens to and when it yields.
const platform = vi.hoisted(() => ({ mobile: true }));
vi.mock("../inputCapability", () => ({ touchPrimary: () => platform.mobile }));

const frames: FrameRequestCallback[] = [];
const cleanups: (() => void)[] = [];
beforeEach(() => {
  frames.length = 0;
  window.requestAnimationFrame = (cb: FrameRequestCallback) => { frames.push(cb); return frames.length; };
  window.cancelAnimationFrame = () => {};
  Object.defineProperty(window, "innerHeight", { value: 800, configurable: true });
  Object.defineProperty(window, "visualViewport", { value: Object.assign(new EventTarget(), { height: 520, offsetTop: 0 }), configurable: true });
});
afterEach(() => { cleanups.splice(0).forEach(done => done()); vi.restoreAllMocks(); document.body.innerHTML = ""; applyAndroidInterfaceZoom(1); });
const flush = (n = 8) => { for (let i = 0; i < n; i++) frames.splice(0).forEach(cb => cb(0)); };

async function setup({ top = 40, height = 30, pins = 0, zoom = 1, toolbarTop = 480 } = {}) {
  applyAndroidInterfaceZoom(zoom);
  const { render } = await import("solid-js/web");
  const bridge = await import("../editorCommandBridge");
  const { MobileKeyboardToolbar, NATIVE_IME_EVENT } = await import("./MobileKeyboardToolbar");
  const scroller = document.createElement("main");
  scroller.className = "main-content";
  document.body.appendChild(scroller);
  scroller.scrollTop = 1000;
  Object.defineProperty(scroller, "clientHeight", { value: 400 });
  scroller.getBoundingClientRect = () => ({ top: 100 } as DOMRect);
  let parent: HTMLElement = scroller;
  for (let i = 0; i < pins; i++) {
    const ancestor = document.createElement("div");
    ancestor.setAttribute("data-sticky-parent", "");
    const ownRow = document.createElement("div");
    ownRow.className = "block-main";
    ownRow.getBoundingClientRect = () => ({ bottom: 60 } as DOMRect);
    ancestor.append(ownRow); parent.append(ancestor); parent = ancestor;
  }
  const block = document.createElement("div");
  block.className = "ls-block";
  parent.append(block);
  const editors: (() => void)[] = [];
  function mountEditor(top: number, height: number, baseScroll = 1000) {
    const view = new EditorView({
      parent: block,
      state: EditorState.create({ doc: "a line", extensions: [EditorView.contentAttributes.of({ "data-cm6-editor": "b1" })] }),
    });
    const editor = view.contentDOM;
    editor.style.setProperty("--ls-block-line-height", "20px");
    const shift = () => (scroller.scrollTop - baseScroll) * zoom;
    const caretRow = () => ({ left: 0, right: 1, top: top - shift(), bottom: top + 20 * zoom - shift() });
    vi.spyOn(view, "hasFocus", "get").mockImplementation(() => document.activeElement === editor);
    vi.spyOn(view, "coordsAtPos").mockImplementation(caretRow);
    editor.getBoundingClientRect = () => ({ top: top - shift(), bottom: top + height * zoom - shift(), height: height * zoom } as DOMRect);
    const caretReveal = cm6CaretReveal(view);
    editors.push(() => { caretReveal.cancel(); view.destroy(); });
    return { view, editor, caretRow };
  }
  const { view, editor, caretRow } = mountEditor(top, height);
  const child = document.createElement("div");
  child.textContent = "First child";
  block.append(child);
  child.getBoundingClientRect = () => ({ top: editor.getBoundingClientRect().bottom + 4 * zoom, bottom: editor.getBoundingClientRect().bottom + 24 * zoom } as DOMRect);
  editor.focus();
  const unregister = bridge.registerFocusedEditorCommandBridge({ blockId: "b1", dispatch: () => true, blur() {} });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(MobileKeyboardToolbar, host);
  const toolbar = host.querySelector<HTMLElement>("[data-mobile-keyboard-toolbar]")!;
  toolbar.getBoundingClientRect = () => ({ top: toolbarTop, bottom: toolbarTop + 40, height: 40 } as DOMRect);
  await Promise.resolve();
  const ime = (visible: boolean) => window.dispatchEvent(new CustomEvent(NATIVE_IME_EVENT, { detail: visible }));
  let disposed = false;
  const done = () => { if (disposed) return; disposed = true; unregister(); dispose(); editors.forEach(stop => stop()); };
  cleanups.push(done);
  return { scroller, editor, view, child, caretRow, mountEditor, ime, done };
}

// Both engines anchor the caret row, independently of the editor's height.
const ABOVE_TOOLBAR = 480 - 3 * 20;
it.each([1, 1.25])("opening the keyboard leaves a short CM6 parent's first child visible at zoom %d, including below pins", async zoom => {
  for (const height of [20, 40]) for (const pins of [0, 2]) {
    const f = await setup({ top: 440, height, pins, zoom });
    try {
      f.ime(true); flush();
      expect(f.caretRow().bottom).toBe(480 - 3 * 20 * zoom);
      expect(f.child.getBoundingClientRect().bottom).toBeLessThan(480);
      expect(f.caretRow().top).toBeGreaterThan(100 + pins * 30 * zoom);
      const settled = f.scroller.scrollTop;
      f.editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertFromPaste" }));
      flush();
      expect(f.scroller.scrollTop).toBe(settled);
    } finally { f.done(); f.scroller.remove(); }
  }
});

it("a tall CM6 block's caret above the three-line band stays put", async () => {
  const f = await setup({ top: 220, height: 800, pins: 2 });
  f.ime(true); flush();
  expect(f.scroller.scrollTop).toBe(1000);
});

it("pins take priority when the keyboard leaves less than three body lines", async () => {
  const f = await setup({ top: 440, pins: 2, toolbarTop: 220 });
  f.ime(true); flush();
  expect(f.caretRow().top).toBe(100 + 2 * 30 + 8);
  expect(f.caretRow().bottom).toBeLessThan(220);
});

it("CM6 focus handoff and paste with the keyboard up use the same caret band", async () => {
  const f = await setup();
  f.ime(true); flush();
  const next = f.mountEditor(440, 40, f.scroller.scrollTop);
  next.editor.focus();
  await new Promise(resolve => setTimeout(resolve, 0)); flush();
  expect(next.caretRow().bottom).toBe(ABOVE_TOOLBAR);
  next.editor.dispatchEvent(new KeyboardEvent("keydown", { key: "v", bubbles: true }));
  f.scroller.scrollTop -= 80;
  next.editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertFromPaste" }));
  flush();
  expect(next.caretRow().bottom).toBe(ABOVE_TOOLBAR);
});

it("the mount-time reveal runs first and the keyboard leaves a visible CM6 caret alone", async () => {
  const f = await setup();
  f.ime(true);
  flush();
  // the settled reveal brought the caret line (y 40) down to the scroller top (108); the keyboard rule keeps it
  expect(f.scroller.scrollTop).toBe(1000 - 68);
  f.done();
});

it("follows a scroll that lands after the initial settle (scroller, window, visual viewport)", async () => {
  for (const target of ["scroller", "window", "visual"] as const) {
    const f = await setup();
    f.ime(true);
    flush();
    f.scroller.scrollTop += 600; // the WebView's late focus scroll takes the caret line off the top
    const source = target === "scroller" ? f.scroller : target === "window" ? window : window.visualViewport!;
    source.dispatchEvent(new Event("scroll"));
    flush();
    expect(f.caretRow().bottom, target).toBe(ABOVE_TOOLBAR);
    expect(f.child.getBoundingClientRect().bottom, target).toBeLessThan(480);
    f.done();
  }
});

it.each(["wheel", "touchstart", "pointerdown", "keydown"])("yields to deliberate %s input", async type => {
  const f = await setup();
  f.ime(true);
  flush();
  const settled = f.scroller.scrollTop;
  f.scroller.dispatchEvent(new Event(type));
  f.scroller.scrollTop += 600;
  f.scroller.dispatchEvent(new Event("scroll"));
  flush();
  expect(f.scroller.scrollTop).toBe(settled + 600);
  f.done();
});

it("stops following when the editor loses focus", async () => {
  const f = await setup();
  f.ime(true);
  flush();
  const settled = f.scroller.scrollTop;
  f.editor.blur();
  f.scroller.scrollTop += 600;
  f.scroller.dispatchEvent(new Event("scroll"));
  flush();
  expect(f.scroller.scrollTop).toBe(settled + 600);
  f.done();
});

it("stops following when the keyboard closes with the editor still focused", async () => {
  const f = await setup();
  f.ime(true);
  flush();
  const settled = f.scroller.scrollTop;
  f.ime(false);
  f.scroller.scrollTop += 600;
  f.scroller.dispatchEvent(new Event("scroll"));
  flush();
  expect(f.scroller.scrollTop).toBe(settled + 600);
  f.done();
});

/** A browser fires `scroll` after a script writes scrollTop; jsdom does not. */
function scrollWriteFiresScroll(scroller: HTMLElement) {
  let top = scroller.scrollTop;
  Object.defineProperty(scroller, "scrollTop", {
    configurable: true,
    get: () => top,
    set: (v: number) => { if (v !== top) { top = v; scroller.dispatchEvent(new Event("scroll")); } },
  });
}

it("a correction's own scroll event settles at once: one more check, no further scroll, no loop", async () => {
  const f = await setup();
  scrollWriteFiresScroll(f.scroller);
  f.ime(true);
  flush();
  f.scroller.scrollTop += 600; // late focus scroll: caret off the top
  const before = vi.mocked(f.view.coordsAtPos).mock.calls.length;
  flush(50); // the correction fires its own scroll event; every follow-up check reads the caret and finds it placed
  expect(f.caretRow().bottom).toBe(ABOVE_TOOLBAR);
  expect(vi.mocked(f.view.coordsAtPos).mock.calls.length - before).toBeLessThanOrEqual(3);
  expect(frames).toHaveLength(0);
  f.done();
});

it("repeated scroll events once the caret is placed change nothing", async () => {
  const f = await setup();
  f.ime(true);
  flush();
  const settled = f.scroller.scrollTop;
  const reads = vi.mocked(f.view.coordsAtPos).mock.calls.length;
  for (let i = 0; i < 5; i++) { f.scroller.dispatchEvent(new Event("scroll")); flush(); }
  expect(f.scroller.scrollTop).toBe(settled);
  // one rAF-coalesced re-check per scroll event, bounded; never a polling loop
  expect(vi.mocked(f.view.coordsAtPos).mock.calls.length - reads).toBeLessThanOrEqual(5);
  expect(frames).toHaveLength(0);
  f.done();
});

it("leaves a caret line that is already inside the visible band alone", async () => {
  const f = await setup();
  f.scroller.scrollTop = 1000 - 240; // caret bottom 300, already above the three-line band
  const before = f.scroller.scrollTop;
  f.ime(true);
  flush();
  expect(f.scroller.scrollTop).toBe(before);
  const settled = f.scroller.scrollTop;
  f.scroller.dispatchEvent(new Event("scroll"));
  flush();
  expect(f.scroller.scrollTop).toBe(settled);
  f.done();
});

it("a CM6 editor without a registered caret row (unmounted) is left alone", async () => {
  const f = await setup();
  f.ime(true);
  flush();
  const settled = f.scroller.scrollTop;
  f.view.destroy();
  f.scroller.dispatchEvent(new Event("scroll"));
  flush();
  expect(f.scroller.scrollTop).toBe(settled);
  f.done();
});
