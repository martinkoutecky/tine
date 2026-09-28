// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { createPaneRouter } from "./router";

afterEach(() => vi.unstubAllGlobals());

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
