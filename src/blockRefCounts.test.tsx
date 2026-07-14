import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { bumpDataRev } from "./ui";

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
});
