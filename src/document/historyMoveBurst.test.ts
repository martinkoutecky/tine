// Master 45279b9c9: holding Mod+Up/Down on a selection is one undo step.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { createMemo, createRoot } from "solid-js";
import { extendSelectionTo, installHistoryRouteContextAdapter, loadFeed, moveSelectionItems, pageByName, resetStore, selectBlock, setRaw, toggleUndoRedoMode, undo, undoTopTag } from "./index";
import { doc } from "./model";
import type { BlockDto } from "../types";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `mb-${++serial}`, raw, collapsed: false, children: [] });
const roots = () => pageByName("P")!.roots.map((id) => doc.byId[id].raw);

beforeAll(() => initParser());
beforeEach(() => {
  serial = 0;
  resetStore();
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(1_000_000);
  loadFeed([{ name: "P", title: "P", kind: "page", pre_block: null, blocks: ["a", "b", "c", "d", "e"].map(block) }]);
});
afterEach(() => { vi.useRealTimers(); });

const tick = (ms: number) => vi.setSystemTime(Date.now() + ms);

describe("a held selection move is one undo step (master 45279b9c9)", () => {
  it("three quick nudges undo together", async () => {
    const [a, b] = pageByName("P")!.roots;
    selectBlock(a);
    extendSelectionTo(b);
    await moveSelectionItems(1); tick(100);
    await moveSelectionItems(1); tick(100);
    await moveSelectionItems(1);
    expect(roots()).toEqual(["c", "d", "e", "a", "b"]);
    undo();
    expect(roots()).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("reversing direction inside the burst is still one step", async () => {
    const [a] = pageByName("P")!.roots;
    selectBlock(a);
    await moveSelectionItems(1); tick(50);
    await moveSelectionItems(1); tick(50);
    await moveSelectionItems(-1);
    expect(roots()).toEqual(["b", "a", "c", "d", "e"]);
    undo();
    expect(roots()).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("a pause of 400 ms starts a new step", async () => {
    const [a] = pageByName("P")!.roots;
    selectBlock(a);
    await moveSelectionItems(1); tick(400);
    await moveSelectionItems(1);
    undo();
    expect(roots()).toEqual(["b", "a", "c", "d", "e"]);
  });

  it("a burst is capped at three seconds", async () => {
    const [a] = pageByName("P")!.roots;
    selectBlock(a);
    await moveSelectionItems(1);
    for (const dir of [1, -1, -1, 1, 1, -1, 1, 1] as const) { tick(350); await moveSelectionItems(dir); }
    expect(roots()).toEqual(["b", "c", "d", "a", "e"]);
    tick(350); // 3,150 ms after the first nudge: a new step
    await moveSelectionItems(1);
    undo();
    expect(roots()).toEqual(["b", "c", "d", "a", "e"]);
    undo();
    expect(roots()).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("another edit in between ends the burst", async () => {
    const [a, , , , e] = pageByName("P")!.roots;
    selectBlock(a);
    await moveSelectionItems(1); tick(50);
    setRaw(e, "E");
    selectBlock(a);
    tick(50);
    await moveSelectionItems(1);
    undo();
    expect(roots()).toEqual(["b", "a", "c", "d", "E"]);
  });

  it("toggling page-only mode refreshes which entry Undo names", async () => {
    const [a] = pageByName("P")!.roots;
    selectBlock(a);
    await moveSelectionItems(1);
    installHistoryRouteContextAdapter({ capture: () => ({ paneId: "main", route: { kind: "page", name: "Other", pageKind: "page" } }), restore: () => false });
    createRoot((dispose) => {
      const tag = createMemo(() => undoTopTag());
      expect(tag()).toBe("move-sel");
      toggleUndoRedoMode(); // page only: nothing on "Other" to undo
      expect(tag()).toBeNull();
      toggleUndoRedoMode();
      expect(tag()).toBe("move-sel");
      dispose();
    });
    installHistoryRouteContextAdapter({ capture: () => null, restore: () => false });
  });
});
