import { describe, expect, it } from "vitest";
import { advanceRevision, currentRevision, latestOwner, readOwned, revisionOwner, serializeOwned } from "./owned";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

describe("owned asynchronous completions", () => {
  it("separates newest request ownership by resource key", async () => {
    const scope = {};
    const first = latestOwner(scope, "page-a");
    const other = latestOwner(scope, "page-b");
    const pending = deferred<number>();
    const result = readOwned(first, pending.promise);
    const newest = latestOwner(scope, "page-a");
    pending.resolve(1);
    expect(await result).toEqual({ kind: "stale" });
    expect(newest()).toBe(true);
    expect(other()).toBe(true);
  });

  it("preserves a current failure and discards a stale failure", async () => {
    const scope = {};
    const current = latestOwner(scope, "one");
    await expect(readOwned(current, Promise.reject(new Error("disk failed")))).rejects.toThrow("disk failed");
    const old = latestOwner(scope, "one");
    const pending = deferred<number>();
    const result = readOwned(old, pending.promise);
    latestOwner(scope, "one");
    pending.reject(new Error("old disk failed"));
    await expect(result).resolves.toEqual({ kind: "stale" });
  });

  it("tracks device write revisions without touching another preference", () => {
    const first = {}, second = {};
    const before = revisionOwner(first, currentRevision(first));
    advanceRevision(first);
    expect(before()).toBe(false);
    expect(revisionOwner(first, 1)()).toBe(true);
    expect(revisionOwner(second, 0)()).toBe(true);
  });

  it("runs one resource's writes in order and reports each current failure", async () => {
    const key = {};
    const first = deferred<number>();
    const started: number[] = [];
    const a = serializeOwned(key, () => true, () => { started.push(1); return first.promise; });
    const b = serializeOwned(key, () => true, async () => { started.push(2); throw new Error("second write failed"); });
    await Promise.resolve();
    expect(started).toEqual([1]);
    first.resolve(7);
    expect(await a).toEqual({ kind: "current", value: 7 });
    await expect(b).rejects.toThrow("second write failed");
    expect(started).toEqual([1, 2]);
  });
});
