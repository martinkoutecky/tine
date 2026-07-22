import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { bumpDataRev } from "./ui";
import { setDoc } from "./store";

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
  it("refetches the count map after a saved block reference lands", async () => {
    let snapshot: Record<string, number> = {};
    const getCounts = vi
      .spyOn(backend(), "getBlockRefCounts")
      .mockImplementation(async () => ({ ...snapshot }));
    const { blockRefCount } = await import("./blockRefCounts");

    await waitUntil(() => getCounts.mock.calls.length >= 1);
    expect(blockRefCount("target-block")).toBe(0);

    snapshot = { "target-block": 1 };
    bumpDataRev();

    await waitUntil(() => getCounts.mock.calls.length >= 2);
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
        path: "pages/Target page.md",
      }],
      feed: ["Target page"],
      loaded: true,
    });
    const getCounts = vi
      .spyOn(backend(), "getBlockRefCounts")
      .mockResolvedValue({ [durable]: 2 });
    const { blockRefCount } = await import("./blockRefCounts");

    bumpDataRev();
    await waitUntil(() => getCounts.mock.calls.length >= 1);

    expect(blockRefCount(transient)).toBe(2);
  });
});
