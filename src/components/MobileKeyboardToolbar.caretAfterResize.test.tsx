// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { applyAndroidInterfaceZoom } from "../zoom";

// GH #783 (Android): after the keyboard resized the view, a tapped line could sit above the top of the screen and
// nothing brought it back (the toolbar rule only scrolls down; the mount-time caret reveal had already run).
// jsdom has no layout, so the caret row and the rects are given; the rule under test is that a resize re-checks the caret.
const platform = vi.hoisted(() => ({ mobile: true }));
vi.mock("../inputCapability", () => ({ touchPrimary: () => platform.mobile }));
vi.mock("../editor/caretRows", () => ({ textareaCaretRow: () => ({ top: 0, bottom: 20 }) }));

const frames: FrameRequestCallback[] = [];
beforeEach(() => {
  frames.length = 0;
  window.requestAnimationFrame = (cb: FrameRequestCallback) => { frames.push(cb); return frames.length; };
  window.cancelAnimationFrame = () => {};
  Object.defineProperty(window, "innerHeight", { value: 800, configurable: true });
  Object.defineProperty(window, "visualViewport", { value: Object.assign(new EventTarget(), { height: 520, offsetTop: 0 }), configurable: true });
});
afterEach(() => { document.body.innerHTML = ""; applyAndroidInterfaceZoom(1); });
const flush = (n = 8) => { for (let i = 0; i < n; i++) frames.splice(0).forEach(cb => cb(0)); };

async function setup({ top = 40, height = 30, pins = 0, zoom = 1 } = {}) {
  applyAndroidInterfaceZoom(zoom);
  const { render } = await import("solid-js/web");
  const bridge = await import("../editorCommandBridge");
  const { MobileKeyboardToolbar, NATIVE_IME_EVENT } = await import("./MobileKeyboardToolbar");
  const scroller = document.createElement("main");
  scroller.className = "main-content";
  const editor = document.createElement("textarea");
  editor.className = "block-editor";
  let ancestorHost: HTMLElement = scroller;
  for (let i = 0; i < pins; i++) {
    const ancestor = document.createElement("div");
    ancestor.setAttribute("data-sticky-parent", "");
    const ownRow = document.createElement("div");
    ownRow.className = "block-main";
    ownRow.getBoundingClientRect = () => ({ bottom: 60 } as DOMRect);
    ancestor.append(ownRow); ancestorHost.append(ancestor); ancestorHost = ancestor;
  }
  const block = document.createElement("div");
  block.className = "ls-block";
  const child = document.createElement("div");
  child.textContent = "First child";
  block.append(editor, child); ancestorHost.append(block);
  document.body.appendChild(scroller);
  scroller.scrollTop = 1000;
  Object.defineProperty(scroller, "clientHeight", { value: 400 });
  scroller.getBoundingClientRect = () => ({ top: 100 } as DOMRect);
  // the editor sits 60px above the scroller top (y 40): the line is off screen
  editor.getBoundingClientRect = () => ({ top: top - (scroller.scrollTop - 1000) * zoom, bottom: top + height * zoom - (scroller.scrollTop - 1000) * zoom, height: height * zoom } as DOMRect);
  child.getBoundingClientRect = () => ({ top: editor.getBoundingClientRect().bottom + 4 * zoom, bottom: editor.getBoundingClientRect().bottom + 24 * zoom } as DOMRect);
  editor.focus();
  const unregister = bridge.registerFocusedEditorCommandBridge({ blockId: "b1", dispatch: () => true, blur() {} });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(MobileKeyboardToolbar, host);
  const toolbar = host.querySelector<HTMLElement>("[data-mobile-keyboard-toolbar]")!;
  toolbar.getBoundingClientRect = () => ({ top: 480, bottom: 520, height: 40 } as DOMRect);
  await Promise.resolve();
  const ime = (visible: boolean) => window.dispatchEvent(new CustomEvent(NATIVE_IME_EVENT, { detail: visible }));
  return { scroller, editor, child, ime, done: () => { unregister(); dispose(); } };
}

it.each([1, 1.25])("opening the keyboard leaves a short parent's child visible at zoom %d, including below pins", async zoom => {
  for (const height of [20, 40]) for (const pins of [0, 2]) {
    const f = await setup({ top: 440, height, pins, zoom });
    try {
      f.ime(true); flush();
      expect(f.editor.getBoundingClientRect().top + 20 * zoom).toBe(480 - 3 * 20 * zoom);
      expect(f.child.getBoundingClientRect().bottom).toBeLessThan(480);
      expect(f.editor.getBoundingClientRect().top).toBeGreaterThan(100 + pins * 30 * zoom);
      const settled = f.scroller.scrollTop;
      f.editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertFromPaste" }));
      flush();
      expect(f.scroller.scrollTop).toBe(settled); // paste/typing uses the same anchor
    } finally { f.done(); f.scroller.remove(); }
  }
});

it("a tall block's caret already above the three-line band stays put", async () => {
  const f = await setup({ top: 220, height: 800, pins: 2 });
  try {
    f.ime(true); flush();
    expect(f.scroller.scrollTop).toBe(1000);
  } finally { f.done(); }
});

it("Enter-split or arrow focus handoff and paste while the keyboard is up use the same caret band", async () => {
  const f = await setup();
  try {
    f.ime(true); flush();
    const next = document.createElement("textarea");
    next.className = "block-editor";
    const start = f.scroller.scrollTop;
    next.getBoundingClientRect = () => ({ top: 440 - (f.scroller.scrollTop - start), bottom: 480 - (f.scroller.scrollTop - start), height: 40 } as DOMRect);
    f.editor.replaceWith(next); next.focus();
    await new Promise(resolve => setTimeout(resolve, 0)); flush();
    expect(next.getBoundingClientRect().top + 20).toBe(ABOVE_TOOLBAR);
    // User key input ends the native-scroll watcher, but paste's input still reveals a newly low caret.
    next.dispatchEvent(new KeyboardEvent("keydown", { key: "v", bubbles: true }));
    f.scroller.scrollTop -= 80;
    next.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertFromPaste" }));
    flush();
    expect(next.getBoundingClientRect().top + 20).toBe(ABOVE_TOOLBAR);
  } finally { f.done(); }
});

// GH #783 comment 14: anchor the caret row, leaving three body lines for following content.
const ABOVE_TOOLBAR = 480 - 3 * 20;
it("brings a tapped line above the top back to the three-line keyboard band (GH #783)", async () => {
  const f = await setup();
  f.ime(true);
  flush();
  expect(f.editor.getBoundingClientRect().top + 20).toBe(ABOVE_TOOLBAR);
  expect(f.scroller.scrollTop).toBe(640);
  f.done();
});

it("leaves the page where the reader scrolled it when the keyboard closes with the editor still focused (GH #783)", async () => {
  const f = await setup();
  f.ime(false);
  flush();
  expect(f.scroller.scrollTop).toBe(1000);
  f.done();
});

it("keeps the edited line visible when native scrolling follows the last keyboard resize (GH #783)", async () => {
  const f = await setup();
  f.ime(true);
  window.dispatchEvent(new Event("resize"));
  flush(); // beta.2's one-shot reveal has finished
  f.scroller.scrollTop += 600; // the line leaves over the top
  f.scroller.dispatchEvent(new Event("scroll"));
  flush();
  expect(f.editor.getBoundingClientRect().top + 20).toBe(ABOVE_TOOLBAR);
  f.done();
});

it("rechecks the caret when focus arrives after the keyboard's last resize (GH #783)", async () => {
  const f = await setup();
  f.editor.blur();
  f.ime(true);
  window.dispatchEvent(new Event("resize"));
  flush();
  f.editor.focus();
  await new Promise(resolve => setTimeout(resolve, 0));
  flush();
  expect(f.editor.getBoundingClientRect().top + 20).toBe(ABOVE_TOOLBAR);
  f.done();
});

it.each(["visual", "window"])("follows a late %s viewport pan without another resize (GH #783)", async target => {
  const f = await setup();
  f.ime(true);
  flush();
  f.scroller.scrollTop += 600;
  (target === "visual" ? window.visualViewport! : window).dispatchEvent(new Event("scroll"));
  flush();
  expect(f.editor.getBoundingClientRect().top + 20).toBe(ABOVE_TOOLBAR);
  f.done();
});

it.each(["wheel", "touchstart", "pointerdown", "keydown"])("yields to deliberate %s scrolling after keyboard opening (GH #783)", async type => {
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

it("stops following native scroll when the keyboard closes (GH #783)", async () => {
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
