import { describe, expect, it } from "vitest";
import { flushAll, trackAssetWrite } from "./document";

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void; reject: (reason?: unknown) => void } {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("asset write close barrier", () => {
  it("flushAll waits for a pending tracked asset write", async () => {
    const asset = deferred<string>();
    const tracked = trackAssetWrite(asset.promise);
    let flushed = false;

    const flush = flushAll().then((ok) => {
      flushed = true;
      return ok;
    });
    await Promise.resolve();
    expect(flushed).toBe(false);

    asset.resolve("saved.png");

    await expect(tracked).resolves.toBe("saved.png");
    await expect(flush).resolves.toBe(true);
    expect(flushed).toBe(true);
  });
});
