import { afterEach, expect, it, vi } from "vitest";
import { captureReadingViewport } from "./readingViewportAnchor";
import { changeCodeWrapping, codeWrapping } from "../codeDisplay";
import { backend } from "../backend";

afterEach(() => { document.body.innerHTML = ""; vi.restoreAllMocks(); vi.unstubAllGlobals(); });

function surface() {
  const scroller = document.createElement("div"); scroller.className = "main-content";
  const content = document.createElement("div");
  const block = document.createElement("div"); block.dataset.blockId = "reading";
  content.append(block); scroller.append(content); document.body.append(scroller);
  Object.defineProperty(scroller, "scrollHeight", { value: 1000, configurable: true });
  let height = 0;
  scroller.scrollTop = 100;
  vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue({ top: 0 } as DOMRect);
  vi.spyOn(block, "getBoundingClientRect").mockImplementation(() => ({ top: 30 + height - (scroller.scrollTop - 100), bottom: 80 } as DOMRect));
  return { scroller, content, block, grow: (value: number) => { height = value; } };
}

it("compensates late content resize and releases on a gesture", () => {
  let resize!: ResizeObserverCallback;
  const disconnect = vi.fn(); const observe = vi.fn();
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: ResizeObserverCallback) { resize = callback; }
    observe = observe; disconnect = disconnect;
  });
  const s = surface();
  captureReadingViewport(s.scroller);
  expect(observe).toHaveBeenCalledWith(s.content);
  s.grow(120); resize([], {} as ResizeObserver);
  expect(s.block.getBoundingClientRect().top).toBe(30);
  s.grow(180); resize([], {} as ResizeObserver);
  expect(s.scroller.scrollTop).toBe(280);
  s.scroller.dispatchEvent(new Event("wheel"));
  s.grow(240); resize([], {} as ResizeObserver);
  expect(s.scroller.scrollTop).toBe(280);
  expect(disconnect).toHaveBeenCalled();
});

it("preserves content after layout shrink clamps the old scroll position to the new range", () => {
  const s = surface();
  const anchor = captureReadingViewport(s.content)!;
  Object.defineProperty(s.scroller, "scrollHeight", { value: 80 });
  s.grow(-60);
  s.scroller.scrollTop = 80; // Browser clamps the former 100 to the new maximum.
  anchor.restore();
  expect(s.scroller.scrollTop).toBe(40);
  expect(s.block.getBoundingClientRect().top).toBe(30);
  anchor.release();
});

it.each(["focus", "scroll", "disconnect", "supersede"])("releases ownership after %s", reason => {
  const s = surface();
  const anchor = captureReadingViewport(s.content)!;
  if (reason === "focus") { const input = document.createElement("input"); document.body.append(input); input.focus(); }
  if (reason === "scroll") s.scroller.scrollTop = 130;
  if (reason === "disconnect") s.content.remove();
  if (reason === "supersede") captureReadingViewport(s.content)?.release();
  s.grow(100);
  anchor.restore();
  expect(s.scroller.scrollTop).toBe(reason === "scroll" ? 130 : 100);
  anchor.release();
});

it("wrap changes give the focused editor sole ownership through its autosize frame", async () => {
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => { frames.push(cb); return frames.length; });
  const s = surface();
  const editor = document.createElement("textarea"); s.block.append(editor); editor.focus();
  vi.spyOn(editor, "getBoundingClientRect").mockImplementation(() => ({ top: 50 + (codeWrapping() ? 200 : 0) - (s.scroller.scrollTop - 100) } as DOMRect));
  changeCodeWrapping(false);
  await Promise.resolve();
  for (const frame of frames.splice(0)) frame(0);
  s.scroller.scrollTop = 100;
  const before = editor.getBoundingClientRect().top;
  changeCodeWrapping(true);
  await Promise.resolve();
  for (const frame of frames.splice(0)) frame(0);
  expect(editor.getBoundingClientRect().top).toBe(before);
  expect(document.activeElement).toBe(editor);
  changeCodeWrapping(false);
  await Promise.resolve();
  for (const frame of frames.splice(0)) frame(0);
  expect(editor.getBoundingClientRect().top).toBe(before);
});
