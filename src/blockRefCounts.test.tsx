import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { bumpDataRev, bumpGraphEpoch } from "./graphSession";
import { applyGraphAnswers } from "./graphAnswers";
import { doc, setDoc } from "./document/model";
import { deleteBlock, loadFeed, mergeWithNext, resetStore, resolveBlockRef } from "./document";
import { setToasts, toasts } from "./toasts";
import { initParser } from "./render/parse";

vi.mock("./warmCache", () => ({
  waitForWarmCache: vi.fn(async () => true),
}));

async function waitUntil(predicate: () => boolean): Promise<void> {
  for (let i = 0; i < 50; i++) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  expect(predicate()).toBe(true);
}

afterEach(() => {
  setDoc({ byId: {}, pages: [], feed: [], loaded: false });
  vi.restoreAllMocks();
});

describe("block reference count refresh (GH #154)", () => {
  it("F2 merge uses graph counts for an unloaded page's ref and embed without reading referrers", async () => {
    await initParser();
    const uuid = "12345678-1234-4234-8234-123456789abc";
    const getCounts = vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({ [uuid]: 2 });
    const getPage = vi.spyOn(backend(), "getPage");
    const { captureBlockReferenceCount } = await import("./blockRefCounts");
    getCounts.mockClear(); // Import may start the previous graph's initial request.
    bumpGraphEpoch();
    await captureBlockReferenceCount([uuid]);
    loadFeed([{ name: "Target", kind: "page", title: "Target", pre_block: null, blocks: [
      { id: "keep", raw: "one", collapsed: false, children: [] },
      { id: "gone", raw: `two\nheading:: true\nid:: ${uuid}`, collapsed: false, children: [] },
    ] }]);
    expect(mergeWithNext("keep")).toBe(true);
    expect(doc.byId.keep.raw).toBe(`onetwo\nid:: ${uuid}`);
    expect(resolveBlockRef({ uuid, page: "Target", pageKind: "page" })).toBe("keep");
    expect(getCounts).toHaveBeenCalledTimes(1);
    expect(getPage).not.toHaveBeenCalled();
    resetStore(); setToasts([]);
  });
  it.each([0, 2])("cold deletion computes %i references and offers the edit's Undo only when affected", async (count) => {
    await initParser();
    let finish!: (value: Record<string, number>) => void;
    const getCounts = vi.spyOn(backend(), "getBlockRefCounts").mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    await import("./blockRefCounts");
    getCounts.mockClear(); bumpGraphEpoch(); setToasts([]);
    const durable = "12345678-1234-4234-8234-123456789abc";
    loadFeed([{ name: "Test", kind: "page", title: "Test", pre_block: null, blocks: [
      { id: "target", raw: `Target\nid:: ${durable}`, collapsed: false, children: [] },
    ] }]);
    deleteBlock("target");
    expect(doc.byId.target).toBeUndefined();
    expect(toasts()).toEqual([]);
    finish({ [durable]: count });
    await waitUntil(() => count ? toasts().length === 1 : getCounts.mock.calls.length === 1);
    await Promise.resolve(); await Promise.resolve();
    if (count) {
      expect(toasts()[0].message).toBe("2 references are now broken");
      toasts()[0].action!.run();
      expect(doc.byId.target.raw).toContain(durable);
    } else expect(toasts()).toEqual([]);
    resetStore(); setToasts([]);
  });

  it("captures a real count while the initial map is cold and shares its request", async () => {
    let finish!: (value: Record<string, number>) => void;
    const getCounts = vi.spyOn(backend(), "getBlockRefCounts").mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    const { captureBlockReferenceCount } = await import("./blockRefCounts");
    getCounts.mockClear();
    bumpGraphEpoch();
    const captured = captureBlockReferenceCount(["cold", "other", "cold"]);
    expect(captured).toBeInstanceOf(Promise);
    await waitUntil(() => getCounts.mock.calls.length === 1);
    finish({ cold: 3, other: 2 });
    expect(await captured).toBe(5);
    expect(captureBlockReferenceCount(["absent"])).toBe(0);
    expect(getCounts).toHaveBeenCalledTimes(1);
  });

  it("never uses another graph's count for a pending edit", async () => {
    let finish!: (value: Record<string, number>) => void;
    vi.spyOn(backend(), "getBlockRefCounts").mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    const { captureBlockReferenceCount } = await import("./blockRefCounts");
    bumpGraphEpoch();
    const captured = captureBlockReferenceCount(["cold"]);
    const finishOld = finish;
    bumpGraphEpoch();
    finishOld({ cold: 99 });
    expect(await captured).toBeUndefined();
  });

  it("updates the count map from the native save signal after a block reference lands", async () => {
    let snapshot: Record<string, number> = {};
    const getCounts = vi
      .spyOn(backend(), "getBlockRefCounts")
      .mockImplementation(async () => ({ ...snapshot }));
    const { blockRefCount } = await import("./blockRefCounts");

    bumpGraphEpoch();

    await waitUntil(() => getCounts.mock.calls.length >= 1);
    expect(blockRefCount("target-block")).toBe(0);

    applyGraphAnswers({ rev: "2", inventoryChanged: false, blockRefCounts: { "target-block": 1 } });
    bumpDataRev();

    await waitUntil(() => blockRefCount("target-block") === 1);
    expect(getCounts).toHaveBeenCalledTimes(1);
    expect(blockRefCount("target-block")).toBe(1);
  });

  it("reads two referrers under a freshly assigned durable id while the live key stays transient", async () => {
    const durable = "12345678-1234-4234-8234-123456789abc";
    const transient = "bfresh-target";
    setDoc({
      byId: {
        [transient]: {
          id: transient,
          raw: `Fresh target\nid:: ${durable}`,
          collapsed: false,
          parent: null,
          page: "Target page",
          children: [],
        },
      },
      pages: [{
        name: "Target page",
        kind: "page",
        title: "Target page",
        preBlock: null,
        roots: [transient],
        format: "md",
        readOnly: false,
        guide: false,
        id: "pages/Target page.md",
      }],
      feed: ["Target page"],
      loaded: true,
    });
    const getCounts = vi
      .spyOn(backend(), "getBlockRefCounts")
      .mockResolvedValue({ [durable]: 2 });
    const { blockRefCount } = await import("./blockRefCounts");

    bumpGraphEpoch();
    await waitUntil(() => getCounts.mock.calls.length >= 1);

    expect(blockRefCount(transient)).toBe(2);
  });
});
