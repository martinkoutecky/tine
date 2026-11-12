// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createPaneRouter } from "./router";

// GH #848: a reference read can answer long after a page shows, so coming back to a deep position keeps
// pursuing it while the page grows, and the row in view wins over the old pixel offset.
let frames: FrameRequestCallback[] = [];
let scroller: HTMLDivElement;
let height = 800;

async function runFrames(count: number) {
  for (let i = 0; i < count; i++) {
    for (const frame of frames.splice(0)) frame(0);
    await Promise.resolve();
    await Promise.resolve();
  }
}

function leaveDeepPage() {
  const router = createPaneRouter(`restore-test-${Math.random()}`);
  router.setScrollerElement(scroller);
  router.openPage("Deep", "page");
  height = 20_000;
  scroller.scrollTop = 9_000;
  router.openPage("Other", "page");
  height = 800;
  scroller.scrollTop = 0;
  router.goBack();
  router.restoreScrollFor(router.route());
  return router;
}

beforeEach(() => {
  frames = [];
  height = 800;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  scroller = document.createElement("div");
  scroller.className = "main-content";
  Object.defineProperty(scroller, "scrollHeight", { get: () => height });
  Object.defineProperty(scroller, "clientHeight", { get: () => 800 });
  document.body.append(scroller);
});

afterEach(() => {
  vi.unstubAllGlobals();
  scroller.remove();
  delete (document as { elementFromPoint?: unknown }).elementFromPoint;
});

it("reaches a deep position when the page grows long after it showed", async () => {
  leaveDeepPage();
  await runFrames(200); // more than three seconds of frames with a short page
  expect(scroller.scrollTop).toBe(0);
  height = 20_000;
  await runFrames(1);
  expect(scroller.scrollTop).toBe(9_000);
});

it("puts the row that was in view back when the rows above it changed height", async () => {
  const row = document.createElement("div");
  row.dataset.blockId = "reading-here";
  scroller.append(row);
  (document as { elementFromPoint?: unknown }).elementFromPoint = () => row;
  let rowTopInContent = 9_010;
  row.getBoundingClientRect = () => ({ top: rowTopInContent - scroller.scrollTop } as DOMRect);
  leaveDeepPage();
  height = 20_000;
  rowTopInContent = 9_510; // a group above measured 500 px taller than its reservation
  await runFrames(2);
  expect(scroller.scrollTop).toBe(9_500);
});

it("stops pursuing on the first user gesture", async () => {
  leaveDeepPage();
  await runFrames(2);
  document.dispatchEvent(new Event("wheel"));
  height = 20_000;
  await runFrames(5);
  expect(scroller.scrollTop).toBe(0);
});

it("does not take the key press that asked for the navigation as a gesture", async () => {
  leaveDeepPage();
  document.dispatchEvent(new Event("keydown")); // still being dispatched when restoration starts
  height = 20_000;
  await runFrames(2);
  expect(scroller.scrollTop).toBe(9_000);
});

it("ends once position and height have held, so a later user scroll is left alone", async () => {
  leaveDeepPage();
  height = 20_000;
  await runFrames(40);
  expect(frames).toHaveLength(0);
});
