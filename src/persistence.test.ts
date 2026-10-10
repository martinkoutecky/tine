import { afterEach, describe, expect, it } from "vitest";
import { flushAll, resetStore, trackAssetWrite } from "./document";
import { bindTestHost } from "./document/host/wiring.test.support";

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void; reject: (reason?: unknown) => void } {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

afterEach(() => resetStore());

describe("asset write close barrier", () => {
  it("flushAll waits for a pending tracked asset write", async () => {
    // Closing, switching and printing all run with the graph's host bound.
    await bindTestHost();
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
