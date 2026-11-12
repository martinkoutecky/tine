// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createPaneRouter } from "./router";
import { pursueScroll } from "./routeScrollAnchor";
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
  scroller.dispatchEvent(new Event("scroll"));
  router.goForward();
  router.restoreScrollFor(router.route());
  await runFrames(1);
  router.goBack();
  router.restoreScrollFor(router.route());
  await runFrames(2);
  expect(scroller.scrollTop).toBe(2_000);
});

it("remembers the reader's new position after a background tab closes during the restore", async () => {
  const router = await leaveDeepPage();
  const shown = router.activeId();
  router.openPageInNewTab("Background", "page");
  const background = router.tabs().find((tab) => tab.id !== shown)!.id;
  await router.closeTab(background);
  height = 20_000;
  await runFrames(3); // the restore reaches its position after the unrelated tab change
  expect(scroller.scrollTop).toBe(9_000);
  document.dispatchEvent(new Event("wheel"));
  scroller.scrollTop = 2_000;
  scroller.dispatchEvent(new Event("scroll"));
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

for (const [what, gesture] of [
  ["Alt before Alt+Right", () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Alt" }))],
  ["a click on a navigation control", () => document.dispatchEvent(new Event("pointerdown"))],
] as const) {
  it(`keeps the saved position when ${what} comes while the returning page is still loading`, async () => {
    const router = await leaveDeepPage();
    await runFrames(1);
    gesture();
    router.goForward();
    router.restoreScrollFor(router.route());
    await runFrames(1);
    router.goBack();
    height = 20_000;
    router.restoreScrollFor(router.route());
    await runFrames(2);
    expect(scroller.scrollTop).toBe(9_000);
  });
}

it("keeps the saved position when Alt is pressed in a workspace window while the returning page is loading", async () => {
  const frame = document.createElement("iframe");
  document.body.append(frame);
  const win = frame.contentWindow! as Window & typeof globalThis;
  (win as { requestAnimationFrame: unknown }).requestAnimationFrame = (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; };
  const main = scroller;
  scroller = win.document.createElement("div");
  Object.defineProperty(scroller, "scrollHeight", { get: () => height });
  Object.defineProperty(scroller, "clientHeight", { get: () => 800 });
  win.document.body.append(scroller);
  try {
    const router = await leaveDeepPage();
    await runFrames(1);
    win.document.dispatchEvent(new win.KeyboardEvent("keydown", { key: "Alt" }));
    height = 20_000;
    await runFrames(2);
    expect(scroller.scrollTop).toBe(9_000);
    void router;
  } finally { scroller.remove(); scroller = main; frame.remove(); }
});

it("keeps at most one interrupted restore waiting for the reader's scroll on a pane scroller", async () => {
  const waiting = new Set<EventListenerOrEventListenerObject>();
  const add = scroller.addEventListener.bind(scroller);
  const remove = scroller.removeEventListener.bind(scroller);
  scroller.addEventListener = ((type: string, listener: EventListenerOrEventListenerObject, options?: AddEventListenerOptions) => {
    if (type === "scroll") waiting.add(listener);
    add(type, listener, options);
  }) as typeof scroller.addEventListener;
  scroller.removeEventListener = ((type: string, listener: EventListenerOrEventListenerObject, options?: EventListenerOptions) => {
    if (type === "scroll") waiting.delete(listener);
    remove(type, listener, options);
  }) as typeof scroller.removeEventListener;
  height = 800; // a short loading view: every visit is interrupted before it can position
  for (let visit = 0; visit < 100; visit++) {
    pursueScroll({ target: 9_000, anchor: undefined, scroller: () => scroller, current: () => true, extend: async () => false, onPositioned: () => {} });
    await runFrames(1);
    document.dispatchEvent(new Event("pointerdown")); // Back/Forward clicks, no scrolling
  }
  expect(waiting.size).toBeLessThanOrEqual(1);
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

it("a stable route() read searches no tabs, however many are open", () => {
  const router = createPaneRouter(`restore-test-${Math.random()}`);
  for (let i = 0; i < 300; i++) router.openPageInNewTab(`Tab ${i}`, "page", undefined, true);
  const search = vi.spyOn(Array.prototype, "find");
  const searchIndex = vi.spyOn(Array.prototype, "findIndex");
  try {
    for (let i = 0; i < 1_000; i++) router.route();
    expect(search.mock.calls.length + searchIndex.mock.calls.length).toBe(0);
  } finally { search.mockRestore(); searchIndex.mockRestore(); }
});
