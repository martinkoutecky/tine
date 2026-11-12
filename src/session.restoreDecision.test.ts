// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { resetStore } from "./document";
import { mainRouter, resetPaneLayoutToSingle } from "./panes";
import { buildPersistedSession, restoreSession } from "./session";
import { activeWorkspaceId, resetWorkspacesForTest } from "./workspaces";
import type { PaneSnapshot } from "./router";

let frames: FrameRequestCallback[];
let scroller: HTMLDivElement;

const query: PaneSnapshot = {
  tabs: [
    { history: [{ kind: "journals" }], pos: 0, pinned: false },
    { history: [{ kind: "query", id: "saved-query", sourceKind: "search", source: "Overflow", presentation: "table" }], pos: 0, pinned: false },
  ],
  activeIndex: 1,
};

function savedQuery() {
  return { ...buildPersistedSession(), ...query, workspaceId: "saved",
    layout: { kind: "pane", paneId: "main", ...query } };
}

async function positionJournals(offset = 0) {
  const router = mainRouter();
  router.restoreScrollFor(router.route());
  for (const frame of frames.splice(0)) frame(0);
  await Promise.resolve();
  scroller.scrollTop = offset;
  expect(router.snapshot().scrolls).toEqual([offset]);
}

beforeEach(() => {
  resetStore();
  resetWorkspacesForTest();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "journals" }], pos: 0, pinned: false }], activeIndex: 0 });
  frames = [];
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  scroller = document.createElement("div");
  Object.defineProperty(scroller, "scrollHeight", { value: 2000 });
  Object.defineProperty(scroller, "clientHeight", { value: 800 });
  document.body.append(scroller);
  mainRouter().setScrollerElement(scroller);
});

afterEach(() => {
  mainRouter().setScrollerElement(null);
  scroller.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetWorkspacesForTest();
  resetStore();
});

it.each([0, 120])("restores the query after initial layout, but respects a user scroll (%s)", async (offset) => {
  const saved = savedQuery();
  let finish!: (raw: string) => void;
  const load = vi.spyOn(backend(), "loadSession").mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const registry = vi.spyOn(backend(), "loadWorkspaces").mockResolvedValue(JSON.stringify({
    version: 1, activeId: "saved", workspaces: [{ id: "saved", name: "", blob: saved }],
  }));
  const pending = restoreSession();
  await positionJournals(offset);
  finish(JSON.stringify(saved));
  await pending;
  expect(load).toHaveBeenCalledTimes(1);
  if (offset === 0) {
    expect(mainRouter().route()).toMatchObject({ kind: "query", source: "Overflow", presentation: "table" });
    expect(activeWorkspaceId()).toBe("saved");
    expect(registry).toHaveBeenCalledTimes(1);
  } else {
    expect(mainRouter().route()).toEqual({ kind: "journals" });
    expect(activeWorkspaceId()).toBe("");
    expect(registry).not.toHaveBeenCalled();
  }
});

it("recovers a parked query when initial layout finishes during the registry read", async () => {
  const saved = savedQuery();
  vi.spyOn(backend(), "loadSession").mockResolvedValue(null);
  let finish!: (raw: string) => void;
  vi.spyOn(backend(), "loadWorkspaces").mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const pending = restoreSession();
  await vi.waitFor(() => expect(finish).toBeDefined());
  await positionJournals();
  finish(JSON.stringify({ version: 1, activeId: "saved", workspaces: [{ id: "saved", name: "", blob: saved }] }));
  await pending;
  expect(mainRouter().route()).toMatchObject({ kind: "query", source: "Overflow", presentation: "table" });
  expect(activeWorkspaceId()).toBe("saved");
});
