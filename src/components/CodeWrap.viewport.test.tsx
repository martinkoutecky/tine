import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { changeCodeWrapping, codeWrapping } from "../codeDisplay";
import { captureReadingViewport } from "./readingViewportAnchor";
import { registerWindow } from "../windowRealm";

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

it.each([false, true])("shrink clamp retains the viewport (focused editor: %s)", async focused => {
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => { frames.push(cb); return frames.length; });
  changeCodeWrapping(true);
  const scroller = document.createElement("div"); scroller.className = "main-content";
  const block = document.createElement("div"); block.dataset.blockId = "reading";
  const editor = document.createElement("textarea");
  block.append(editor); scroller.append(block); document.body.append(scroller);
  if (focused) editor.focus();
  Object.defineProperty(scroller, "clientHeight", { get: () => 200 });
  Object.defineProperty(scroller, "scrollHeight", { get: () => codeWrapping() ? 1000 : 400 });
  let scroll = 580;
  Object.defineProperty(scroller, "scrollTop", {
    get: () => scroll = Math.min(scroll, scroller.scrollHeight - scroller.clientHeight),
    set: value => { scroll = Math.max(0, Math.min(value, scroller.scrollHeight - scroller.clientHeight)); },
  });
  vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue({ top: 0 } as DOMRect);
  const rect = () => ({ top: (codeWrapping() ? 600 : 80) - scroller.scrollTop, bottom: (codeWrapping() ? 640 : 120) - scroller.scrollTop } as DOMRect);
  vi.spyOn(block, "getBoundingClientRect").mockImplementation(rect);
  vi.spyOn(editor, "getBoundingClientRect").mockImplementation(rect);
  const before = rect().top;
  changeCodeWrapping(false);
  await Promise.resolve();
  for (const frame of frames.splice(0)) frame(0);
  expect(rect().top).toBe(before);
});

it("wrap retains a visible descendant within a long outline parent", () => {
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  changeCodeWrapping(false);
  const scroller = document.createElement("div"); scroller.className = "main-content";
  const parent = document.createElement("div"); parent.dataset.blockId = "parent";
  const code = document.createElement("div"); code.dataset.blockId = "code-above";
  const marker = document.createElement("div"); marker.dataset.blockId = "visible-marker";
  parent.append(code, marker); scroller.append(parent); document.body.append(scroller);
  scroller.scrollTop = 500;
  vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue({ top: 0 } as DOMRect);
  vi.spyOn(parent, "getBoundingClientRect").mockImplementation(() => ({ top: 100 - scroller.scrollTop, bottom: 1000 + (codeWrapping() ? 160 : 0) - scroller.scrollTop } as DOMRect));
  vi.spyOn(code, "getBoundingClientRect").mockImplementation(() => ({ top: 140 - scroller.scrollTop, bottom: 490 + (codeWrapping() ? 160 : 0) - scroller.scrollTop } as DOMRect));
  vi.spyOn(marker, "getBoundingClientRect").mockImplementation(() => ({ top: 520 + (codeWrapping() ? 160 : 0) - scroller.scrollTop, bottom: 560 + (codeWrapping() ? 160 : 0) - scroller.scrollTop } as DOMRect));
  const before = marker.getBoundingClientRect().top;
  changeCodeWrapping(true);
  expect(marker.getBoundingClientRect().top).toBe(before);
});

it.each(["ongoing scrollbar drag", "wheel", "touchstart", "pointerdown", "keydown"])("a %s during late shrink must retain user ownership", gesture => {
  let resize!: ResizeObserverCallback;
  vi.stubGlobal("ResizeObserver", class {
    constructor(cb: ResizeObserverCallback) { resize = cb; }
    observe() {}
    disconnect() {}
  });
  const scroller = document.createElement("div"); scroller.className = "main-content";
  const block = document.createElement("div"); block.dataset.blockId = "reading";
  scroller.append(block); document.body.append(scroller);
  Object.defineProperty(scroller, "scrollHeight", { value: 1000, configurable: true });
  let height = 0;
  scroller.scrollTop = 580;
  vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue({ top: 0 } as DOMRect);
  vi.spyOn(block, "getBoundingClientRect").mockImplementation(() => ({ top: 600 + height - scroller.scrollTop, bottom: 640 + height - scroller.scrollTop } as DOMRect));
  if (gesture === "ongoing scrollbar drag") scroller.dispatchEvent(new Event("pointerdown"));
  const anchor = captureReadingViewport(scroller)!;
  anchor.restore();
  if (gesture === "ongoing scrollbar drag") {
    // A scrollbar drag began before the shortcut, continues while late layout
    // is pending, and then the shrinking scroll range clamps its result.
    scroller.dispatchEvent(new MouseEvent("pointermove", { buttons: 1 }));
    scroller.scrollTop = 900;
  } else scroller.dispatchEvent(new Event(gesture));
  Object.defineProperty(scroller, "scrollHeight", { value: 400 });
  height = -520;
  scroller.scrollTop = 400;
  resize([], {} as ResizeObserver);
  anchor.release();
  expect(scroller.scrollTop).toBe(400);
});

it("enumerates a registered secondary window and skips a removed pane", () => {
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  changeCodeWrapping(false);
  const frame = document.createElement("iframe"); document.body.append(frame);
  const unregister = registerWindow("review-popup", frame.contentWindow!);
  const scroller = frame.contentDocument!.createElement("div"); scroller.className = "main-content";
  const block = frame.contentDocument!.createElement("div"); block.dataset.blockId = "reading";
  scroller.append(block); frame.contentDocument!.body.append(scroller);
  scroller.scrollTop = 100;
  vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue({ top: 0 } as DOMRect);
  vi.spyOn(block, "getBoundingClientRect").mockImplementation(() => ({ top: 120 + (codeWrapping() ? 160 : 0) - scroller.scrollTop, bottom: 160 + (codeWrapping() ? 160 : 0) - scroller.scrollTop } as DOMRect));
  const removed = document.createElement("div"); removed.className = "main-content";
  const staleRead = vi.spyOn(removed, "getBoundingClientRect");
  document.body.append(removed); removed.remove();
  try {
    changeCodeWrapping(true);
    expect(block.getBoundingClientRect().top).toBe(20);
    expect(staleRead).not.toHaveBeenCalled();
    unregister();
    changeCodeWrapping(false);
    expect(scroller.scrollTop).toBe(260);
  } finally { unregister(); }
});

it("does not capture an existing touch fling until scrolling settles", () => {
  vi.useFakeTimers();
  const scroller = document.createElement("div"); scroller.className = "main-content";
  const block = document.createElement("div"); block.dataset.blockId = "reading";
  scroller.append(block); document.body.append(scroller);
  vi.spyOn(block, "getBoundingClientRect").mockReturnValue({ top: 20, bottom: 60 } as DOMRect);
  scroller.dispatchEvent(new Event("touchstart", { bubbles: true }));
  expect(captureReadingViewport(scroller)).toBeUndefined();
  scroller.dispatchEvent(new Event("touchend", { bubbles: true }));
  vi.advanceTimersByTime(150);
  scroller.dispatchEvent(new Event("scroll"));
  vi.advanceTimersByTime(150);
  expect(captureReadingViewport(scroller)).toBeUndefined();
  scroller.dispatchEvent(new Event("scrollend"));
  const anchor = captureReadingViewport(scroller);
  expect(anchor).toBeDefined();
  anchor?.release();
  // Older WebKit without scrollend uses a quiet period instead.
  scroller.dispatchEvent(new Event("touchstart", { bubbles: true }));
  scroller.dispatchEvent(new Event("touchend", { bubbles: true }));
  vi.advanceTimersByTime(250);
  const settled = captureReadingViewport(scroller);
  expect(settled).toBeDefined();
  settled?.release();
});

it("releases for a continuing touch but keeps ownership through pointer hover", () => {
  const scroller = document.createElement("div"); scroller.className = "main-content";
  const block = document.createElement("div"); block.dataset.blockId = "reading";
  scroller.append(block); document.body.append(scroller);
  let height = 0;
  vi.spyOn(block, "getBoundingClientRect").mockImplementation(() => ({ top: 20 + height - scroller.scrollTop, bottom: 60 + height - scroller.scrollTop } as DOMRect));
  const anchor = captureReadingViewport(scroller)!;
  scroller.dispatchEvent(new MouseEvent("pointermove", { buttons: 0 }));
  height = 100; anchor.restore();
  expect(scroller.scrollTop).toBe(100);
  scroller.dispatchEvent(new Event("touchmove"));
  height = 200; anchor.restore();
  expect(scroller.scrollTop).toBe(100);
  anchor.release();
});
