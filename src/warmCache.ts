import { createEffect, createRoot } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";
import { backend } from "./backend";
import { graphEpoch } from "./graphSession";
import { ownedWhen, readOwned, readOwnedResource } from "./owned";

type Unlisten = () => void;
const waits = new Set<() => void>();
clearOnBindingInvalidated(() => {
  for (const retire of [...waits]) retire();
  waits.clear();
});

export interface WarmCacheWaitDeps {
  currentEpoch(): number;
  warmDone(): Promise<boolean>;
  listenWarmCacheDone(cb: () => void): Promise<Unlisten>;
}

const defaultDeps: WarmCacheWaitDeps = {
  currentEpoch: graphEpoch,
  warmDone: () => backend().warmDone(),
  async listenWarmCacheDone(cb) {
    const { listen } = await import("@tauri-apps/api/event");
    return listen("warm-cache-done", () => cb());
  },
};

/** Subscribe to warm-cache-done, then probe readiness for this graph epoch.
 * Return true only when ready in the same epoch. A stale epoch or failed
 * subscription/probe can return false. If the probe reports false and no event
 * arrives, waits until readiness or retirement. Epoch change/store reset resolves
 * false and disposes the listener, including late subscription installation.
 * Setup/event work is O(1); reset cleanup is O(active waits). */
export async function waitForWarmCache(
  epoch = graphEpoch(),
  deps: WarmCacheWaitDeps = defaultDeps
): Promise<boolean> {
  if (epoch !== deps.currentEpoch()) return false;

  let done = false;
  let unlisten: Unlisten | null = null;
  let stopWatching: (() => void) | undefined;
  let retire: (() => void) | undefined;

  const finish = (ready: boolean, resolve: (ready: boolean) => void) => {
    if (done) return;
    done = true;
    if (retire) waits.delete(retire);
    stopWatching?.();
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
    resolve(ready && epoch === deps.currentEpoch());
  };

  return new Promise<boolean>((resolve) => {
    const owner = ownedWhen(() => !done && epoch === deps.currentEpoch());
    retire = () => finish(false, resolve);
    waits.add(retire);
    createRoot((dispose) => {
      stopWatching = dispose;
      createEffect(() => { if (epoch !== deps.currentEpoch()) finish(false, resolve); });
    });
    if (done) return;
    readOwnedResource(owner, deps.listenWarmCacheDone(() => finish(true, resolve)), (u) => u())
      .then((installed) => {
        if (installed.kind === "stale") { finish(false, resolve); return; }
        // Retirement may occur between readOwnedResource's check and this
        // promise continuation. A late installed listener still needs disposal.
        if (!owner()) { installed.value(); finish(false, resolve); return; }
        unlisten = installed.value;
        // Subscribe first, then probe the command so small graphs cannot lose the
        // event/command race. During this warm window block-ref badges stay
        // absent/zero; this does not block first paint.
        void readOwned(owner, deps.warmDone())
          .then((ready) => {
            if (ready.kind === "stale") finish(false, resolve);
            else if (ready.value) finish(true, resolve);
          })
          .catch(() => {
            // Keep waiting for the event; a transient IPC failure must not spin.
          });
      })
      .catch(() => {
        if (!owner()) { finish(false, resolve); return; }
        void readOwned(owner, deps.warmDone())
          .then((ready) => finish(ready.kind === "current" && ready.value, resolve))
          .catch(() => finish(false, resolve));
      });
  });
}
