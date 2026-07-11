import { afterEach, describe, expect, it, vi } from "vitest";
import { resetSharedQueryResultsForTests, sharedQueryResult } from "./queryResultCache";

afterEach(resetSharedQueryResultsForTests);

describe("shared query results", () => {
  it("coalesces identical concurrent pane requests and shares the DTO", async () => {
    const value = [{ page: "P", blocks: [] }];
    const load = vi.fn(async () => value);
    const [a, b] = await Promise.all([
      sharedQueryResult("graph-a:1", "simple:q:rev-2", load),
      sharedQueryResult("graph-a:1", "simple:q:rev-2", load),
    ]);
    expect(load).toHaveBeenCalledTimes(1);
    expect(a).toBe(b);
  });

  it("does not share across revisions or graph epochs", async () => {
    let call = 0;
    const load = vi.fn(async () => ({ call: ++call }));
    await sharedQueryResult("graph-a:1", "simple:q:rev-1", load);
    await sharedQueryResult("graph-a:1", "simple:q:rev-2", load);
    await sharedQueryResult("graph-a:2", "simple:q:rev-2", load);
    expect(load).toHaveBeenCalledTimes(3);
  });

  it("does not let a late old-graph result populate the new scope", async () => {
    let finishOld!: (value: { graph: string }) => void;
    const old = sharedQueryResult(
      "graph-a:1",
      "simple:q:rev-1",
      () => new Promise<{ graph: string }>((resolve) => { finishOld = resolve; }),
    );
    const fresh = { graph: "b" };
    await sharedQueryResult("graph-b:2", "simple:q:rev-1", async () => fresh);
    finishOld({ graph: "a" });
    await old;
    const load = vi.fn(async () => ({ graph: "b-second" }));
    const again = await sharedQueryResult("graph-b:2", "simple:q:rev-1", load);
    expect(load).not.toHaveBeenCalled();
    expect(again).toBe(fresh);
  });

  it("drops failed in-flight work so a retry can run", async () => {
    const fail = vi.fn(async () => { throw new Error("nope"); });
    await expect(sharedQueryResult("graph-a:1", "q", fail)).rejects.toThrow("nope");
    const ok = vi.fn(async () => ({ ok: true }));
    await expect(sharedQueryResult("graph-a:1", "q", ok)).resolves.toEqual({ ok: true });
    expect(ok).toHaveBeenCalledTimes(1);
  });
});
