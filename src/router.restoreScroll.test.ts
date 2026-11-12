// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createPaneRouter } from "./router";
import { createRoot } from "solid-js";

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

async function leaveDeepPage(snapshotWhileLoading = false) {
  const router = createPaneRouter(`restore-test-${Math.random()}`);
  router.setScrollerElement(scroller);
  router.openPage("Deep", "page");
  height = 20_000;
  router.restoreScrollFor(router.route());
  await runFrames(1);
  scroller.scrollTop = 9_000;
  router.openPage("Other", "page");
  height = 800;
  scroller.scrollTop = 0;
  router.restoreScrollFor(router.route());
  await runFrames(1);
  router.goBack();
  if (snapshotWhileLoading) router.snapshot();
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

it("keeps the pane router live after the surface that created it is disposed", () => {
  let router!: ReturnType<typeof createPaneRouter>;
  const dispose = createRoot(dispose => { router = createPaneRouter("retained-pane"); return dispose; });
  router.openPage("Before", "page");
  dispose();
  router.openPage("After", "page");
  expect(router.route()).toMatchObject({ kind: "page", name: "After" });
});

it("reaches a deep position when the page grows long after it showed", async () => {
  await leaveDeepPage();
  await runFrames(200); // more than three seconds of frames with a short page
  expect(scroller.scrollTop).toBe(0);
  height = 20_000;
  await runFrames(1);
  expect(scroller.scrollTop).toBe(9_000);
});

it("keeps the reading position when a session snapshot runs while the returning page is loading", async () => {
  await leaveDeepPage(true);
  height = 20_000;
  await runFrames(2);
  expect(scroller.scrollTop).toBe(9_000);
});

it("does no layout work for a snapshot before the destination owns its scroll position", async () => {
  const router = await leaveDeepPage();
  const layout = vi.spyOn(scroller, "getBoundingClientRect");
  router.snapshot();
  expect(layout).not.toHaveBeenCalled();
  layout.mockRestore();
});

it("keeps the saved position through snapshots before and after the first short layout", async () => {
  const router = await leaveDeepPage();
  router.snapshot();
  await runFrames(2);
  router.snapshot();
  router.openPage("Away again", "page");
  router.restoreScrollFor(router.route());
  await runFrames(1);
  router.goBack();
  router.restoreScrollFor(router.route());
  height = 20_000;
  await runFrames(2);
  expect(scroller.scrollTop).toBe(9_000);
});

it("keeps the position when returning before the intermediate view was positioned", async () => {
  const router = await leaveDeepPage();
  height = 20_000;
  await runFrames(2);
  router.openPage("Still loading", "page");
  height = 800;
  scroller.scrollTop = 0;
  router.goBack();
  router.snapshot();
  router.restoreScrollFor(router.route());
  height = 20_000;
  await runFrames(2);
  expect(scroller.scrollTop).toBe(9_000);
});

it("keeps the saved position when the pane scroller remounts on the same route", async () => {
  const router = await leaveDeepPage();
  height = 20_000;
  await runFrames(2);
  router.snapshot();
  router.setScrollerElement(null);
  scroller.scrollTop = 0;
  router.setScrollerElement(scroller);
  router.snapshot();
  router.restoreScrollFor(router.route());
  await runFrames(2);
  expect(scroller.scrollTop).toBe(9_000);
});

it("restores each tab's own position when the rendered route has the same value", async () => {
  const router = await leaveDeepPage();
  height = 20_000;
  await runFrames(2);
  const firstTab = router.activeId();
  const renderedRoute = router.route();
  router.openPageInNewTab("Deep", "page", undefined, true);
  router.restoreScrollFor(renderedRoute);
  await runFrames(1);
  expect(scroller.scrollTop).toBe(0);
  scroller.scrollTop = 3_000;
  router.setActiveTab(firstTab);
  router.snapshot();
  router.restoreScrollFor(router.route());
  await runFrames(2);
  expect(scroller.scrollTop).toBe(9_000);
});

it("remembers deliberate user scrolling after an interrupted restore", async () => {
  const router = await leaveDeepPage();
  await runFrames(2);
  document.dispatchEvent(new Event("wheel"));
  height = 20_000;
  scroller.scrollTop = 2_000;
  router.goForward();
  router.restoreScrollFor(router.route());
  await runFrames(1);
  router.goBack();
  router.restoreScrollFor(router.route());
  await runFrames(2);
  expect(scroller.scrollTop).toBe(2_000);
});

it("remembers the reader's new position when the page is permanently shorter than the saved offset", async () => {
  const router = await leaveDeepPage();
  const clock = vi.spyOn(performance, "now").mockReturnValue(performance.now() + 11_000);
  await runFrames(1); // the existing pursuit ends with the shorter page
  clock.mockRestore();
  height = 2_000;
  scroller.scrollTop = 500;
  router.goForward();
  router.restoreScrollFor(router.route());
  await runFrames(1);
  router.goBack();
  router.restoreScrollFor(router.route());
  await runFrames(2);
  expect(scroller.scrollTop).toBe(500);
});

it("puts the row that was in view back when the rows above it changed height", async () => {
  const row = document.createElement("div");
  row.dataset.blockId = "reading-here";
  scroller.append(row);
  (document as { elementFromPoint?: unknown }).elementFromPoint = () => row;
  let rowTopInContent = 9_010;
  row.getBoundingClientRect = () => ({ top: rowTopInContent - scroller.scrollTop } as DOMRect);
  await leaveDeepPage();
  height = 20_000;
  rowTopInContent = 9_510; // a group above measured 500 px taller than its reservation
  await runFrames(2);
  expect(scroller.scrollTop).toBe(9_500);
});

it("stops pursuing on the first user gesture", async () => {
  await leaveDeepPage();
  await runFrames(2);
  document.dispatchEvent(new Event("wheel"));
  height = 20_000;
  await runFrames(5);
  expect(scroller.scrollTop).toBe(0);
});

it("does not take the key press that asked for the navigation as a gesture", async () => {
  await leaveDeepPage();
  document.dispatchEvent(new Event("keydown")); // still being dispatched when restoration starts
  height = 20_000;
  await runFrames(2);
  expect(scroller.scrollTop).toBe(9_000);
});

it("ends once position and height have held, so a later user scroll is left alone", async () => {
  await leaveDeepPage();
  height = 20_000;
  await runFrames(40);
  expect(frames).toHaveLength(0);
});
