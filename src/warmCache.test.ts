import { describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { resetStore } from "./document";
import { waitForWarmCache, type WarmCacheWaitDeps } from "./warmCache";

function deps(overrides: Partial<WarmCacheWaitDeps> = {}) {
  let current = 7;
  const listeners: (() => void)[] = [];
  let unlistened = 0;
  const d: WarmCacheWaitDeps = {
    currentEpoch: () => current,
    warmDone: async () => false,
    listenWarmCacheDone: async (cb) => {
      listeners.push(cb);
      return () => {
        unlistened += 1;
      };
    },
    ...overrides,
  };
  return {
    deps: d,
    listeners,
    get unlistened() {
      return unlistened;
    },
    setCurrent(epoch: number) {
      current = epoch;
    },
  };
}

describe("waitForWarmCache", () => {
  it("subscribes before probing warm_done so a prior event is covered", async () => {
    let subscribed = false;
    const h = deps({
      warmDone: async () => {
        expect(subscribed).toBe(true);
        return true;
      },
      listenWarmCacheDone: async (cb) => {
        subscribed = true;
        h.listeners.push(cb);
        return () => {};
      },
    });

    await expect(waitForWarmCache(7, h.deps)).resolves.toBe(true);
  });

  it("resolves from the event when the command initially says not ready", async () => {
    let resolveWarmDone: (ready: boolean) => void = () => {};
    const h = deps({
      warmDone: () => new Promise<boolean>((resolve) => {
        resolveWarmDone = resolve;
      }),
    });

    const ready = waitForWarmCache(7, h.deps);
    await Promise.resolve();
    await Promise.resolve();
    h.listeners[0]();
    resolveWarmDone(false);

    await expect(ready).resolves.toBe(true);
    expect(h.unlistened).toBe(1);
  });

  it.each(["epoch", "reset"])("L17:66: retires an eventless wait on %s", async (kind) => {
    const [epoch, setEpoch] = createSignal(7);
    const h = deps({ currentEpoch: epoch });
    let result: boolean | undefined;
    const waiting = waitForWarmCache(7, h.deps).then((value) => { result = value; });
    await vi.waitFor(() => expect(h.listeners).toHaveLength(1));
    if (kind === "epoch") setEpoch(8); else resetStore();
    await vi.waitFor(() => expect(result).toBe(false), { timeout: 150 });
    await waiting;
    expect(h.unlistened).toBe(1);
  });

  it("discards a wait from an older graph epoch", async () => {
    const h = deps();

    const ready = waitForWarmCache(7, h.deps);
    await Promise.resolve();
    await Promise.resolve();
    h.setCurrent(8);
    h.listeners[0]();

    await expect(ready).resolves.toBe(false);
    expect(h.unlistened).toBe(1);
  });
});
