// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { createPaneRouter } from "./router";
import * as documentStore from "./document";

afterEach(() => vi.unstubAllGlobals());

it("does not scroll a block route after its pane navigates away", () => {
  vi.useFakeTimers();
  const resolve = vi.spyOn(documentStore, "resolveBlockRef").mockReturnValue("block-1");
  const router = createPaneRouter("block-route-test");
  const scroller = document.createElement("div");
  scroller.className = "main-content";
  const block = document.createElement("div");
  block.className = "ls-block";
  block.dataset.blockId = "block-1";
  block.scrollIntoView = vi.fn();
  scroller.append(block);
  document.body.append(scroller);
  router.setScrollerElement(scroller);
  try {
    router.openPageAtBlock("Old", "page", "block-1");
    router.openPage("New", "page");
    vi.advanceTimersByTime(100);
    expect(block.scrollIntoView).not.toHaveBeenCalled();
  } finally { resolve.mockRestore(); scroller.remove(); vi.useRealTimers(); }
});

it("does not apply an old route's scroll restoration after navigation", () => {
  const router = createPaneRouter("scroll-owner-test");
  const scroller = document.createElement("div");
  document.body.appendChild(scroller);
  router.setScrollerElement(scroller);
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  const oldRoute = router.route();
  router.restoreScrollFor(oldRoute);
  router.openPage("New page", "page", { inPlace: true });
  scroller.scrollTop = 37;
  frames.shift()!(0);
  expect(scroller.scrollTop).toBe(37);
});
