// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { cm6CaretReveal } from "../editor/cm6CaretReveal";

// GH #783 follow-up (Android, CM6 editor). The textarea editor follows a keyboard that opens, or a WebView focus scroll
// that finishes, after the initial settle (MobileKeyboardToolbar -> watchCaretReveal with scroll following) and puts the
// caret just above the keyboard toolbar. The CM6 editor is a contenteditable div: the toolbar only did this for a
// textarea, so a late scroll left the CM6 caret under the keyboard. jsdom has no layout: the caret rect and the
// element rects are given; the rules under test are which events the follow listens to and when it yields.
const platform = vi.hoisted(() => ({ mobile: true }));
vi.mock("../inputCapability", () => ({ touchPrimary: () => platform.mobile }));

const frames: FrameRequestCallback[] = [];
beforeEach(() => {
  frames.length = 0;
  window.requestAnimationFrame = (cb: FrameRequestCallback) => { frames.push(cb); return frames.length; };
  window.cancelAnimationFrame = () => {};
  Object.defineProperty(window, "innerHeight", { value: 800, configurable: true });
  Object.defineProperty(window, "visualViewport", { value: Object.assign(new EventTarget(), { height: 520, offsetTop: 0 }), configurable: true });
});
afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });
const flush = (n = 8) => { for (let i = 0; i < n; i++) frames.splice(0).forEach(cb => cb(0)); };

async function setup() {
  const { render } = await import("solid-js/web");
  const bridge = await import("../editorCommandBridge");
  const { MobileKeyboardToolbar, NATIVE_IME_EVENT } = await import("./MobileKeyboardToolbar");
  const scroller = document.createElement("main");
  scroller.className = "main-content";
  document.body.appendChild(scroller);
  scroller.scrollTop = 1000;
  Object.defineProperty(scroller, "clientHeight", { value: 400 });
  scroller.getBoundingClientRect = () => ({ top: 100 } as DOMRect);
  const view = new EditorView({
    parent: scroller,
    state: EditorState.create({ doc: "a line", extensions: [EditorView.contentAttributes.of({ "data-cm6-editor": "b1" })] }),
  });
  const editor = view.contentDOM;
  // the caret line sits 60px above the scroller top (y 40): off screen
  const shift = () => scroller.scrollTop - 1000;
  vi.spyOn(view, "hasFocus", "get").mockImplementation(() => document.activeElement === editor);
  vi.spyOn(view, "coordsAtPos").mockImplementation(() => ({ left: 0, right: 1, top: 40 - shift(), bottom: 60 - shift() }));
  editor.getBoundingClientRect = () => ({ top: 40 - shift(), bottom: 70 - shift(), height: 30 } as DOMRect);
  const caretReveal = cm6CaretReveal(view);
  editor.focus();
  const unregister = bridge.registerFocusedEditorCommandBridge({ blockId: "b1", dispatch: () => true, blur() {} });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(MobileKeyboardToolbar, host);
  const toolbar = host.querySelector<HTMLElement>("[data-mobile-keyboard-toolbar]")!;
  toolbar.getBoundingClientRect = () => ({ top: 480, bottom: 520, height: 40 } as DOMRect);
  await Promise.resolve();
  const ime = (visible: boolean) => window.dispatchEvent(new CustomEvent(NATIVE_IME_EVENT, { detail: visible }));
  return { scroller, editor, view, ime, done: () => { unregister(); dispose(); caretReveal.cancel(); view.destroy(); } };
}

// The editor (y 40..70, short) goes where the textarea's goes: its bottom 8px above the toolbar (y 480).
const ABOVE_TOOLBAR = 480 - 8;
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
    expect(f.editor.getBoundingClientRect().bottom, target).toBe(ABOVE_TOOLBAR);
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
  expect(f.editor.getBoundingClientRect().bottom).toBe(ABOVE_TOOLBAR);
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
  f.scroller.scrollTop = 1000 - 402 + 200 - 200; // line already at its resting place (bottom 472)
  f.ime(true);
  flush();
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
