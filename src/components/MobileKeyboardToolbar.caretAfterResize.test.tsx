// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";

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
  Object.defineProperty(window, "visualViewport", { value: { height: 520, offsetTop: 0, addEventListener: vi.fn(), removeEventListener: vi.fn() }, configurable: true });
});
afterEach(() => { document.body.innerHTML = ""; });
const flush = (n = 8) => { for (let i = 0; i < n; i++) frames.splice(0).forEach(cb => cb(0)); };

it("brings a tapped line that sits above the top back into view after the keyboard resizes the view (GH #783)", async () => {
  const { render } = await import("solid-js/web");
  const bridge = await import("../editorCommandBridge");
  const { MobileKeyboardToolbar, NATIVE_IME_EVENT } = await import("./MobileKeyboardToolbar");
  const scroller = document.createElement("main");
  scroller.className = "main-content";
  const editor = document.createElement("textarea");
  editor.className = "block-editor";
  scroller.appendChild(editor);
  document.body.appendChild(scroller);
  scroller.scrollTop = 1000;
  Object.defineProperty(scroller, "clientHeight", { value: 400 });
  scroller.getBoundingClientRect = () => ({ top: 100 } as DOMRect);
  // the editor sits 60px above the scroller top (y 40): the line is off screen
  editor.getBoundingClientRect = () => ({ top: 40 - (scroller.scrollTop - 1000), bottom: 70 - (scroller.scrollTop - 1000), height: 30 } as DOMRect);
  editor.focus();
  const unregister = bridge.registerFocusedEditorCommandBridge({ blockId: "b1", dispatch: () => true, blur() {} });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(MobileKeyboardToolbar, host);
  const toolbar = host.querySelector<HTMLElement>("[data-mobile-keyboard-toolbar]")!;
  toolbar.getBoundingClientRect = () => ({ top: 480, bottom: 520, height: 40 } as DOMRect);
  await Promise.resolve();
  window.dispatchEvent(new CustomEvent(NATIVE_IME_EVENT, { detail: true }));
  flush();
  // visible top is the scroller top 100 + 8: the line (y 40) moves up the page by 68px
  expect(scroller.scrollTop).toBe(1000 - 68);
  unregister();
  dispose();
});
