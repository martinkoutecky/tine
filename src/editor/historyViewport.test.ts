// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { captureRawHistoryViewport, registerHistoryEditorTarget } from "../editorController";
let cleanups: (() => void)[] = [];
afterEach(() => { cleanups.forEach((cleanup) => cleanup()); cleanups = []; document.body.innerHTML = ""; vi.unstubAllGlobals(); });
it.each(["same", "other-surface", "wheel"])("raw replay respects %s editor ownership", (mode) => {
  let frame!: FrameRequestCallback;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frame = callback; return 1; });
  const scroller = document.createElement("div");
  document.body.append(scroller); scroller.scrollTop = 104;
  const mount = (top: number, surface: string) => {
    const editor = document.createElement("textarea"); scroller.append(editor); editor.focus();
    editor.getBoundingClientRect = () => ({ top } as DOMRect);
    const unregister = registerHistoryEditorTarget({ blockId: "source", owner: null, surface,
      selection: () => ({ start: 0, end: 0 }), focused: () => document.activeElement === editor,
      viewport: () => ({ editor, scroller }) });
    cleanups.push(unregister);
    return () => { unregister(); editor.remove(); };
  };
  const unmount = mount(151, "main");
  const afterReplay = captureRawHistoryViewport("source")!;
  unmount(); mount(47, mode === "other-surface" ? "sidebar" : "main");
  afterReplay();
  if (mode === "wheel") scroller.dispatchEvent(new Event("wheel"));
  frame(0);
  expect(scroller.scrollTop).toBe(mode === "same" ? 0 : 104);
});
