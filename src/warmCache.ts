import { backend } from "./backend";
import { graphEpoch } from "./graphSession";
import { ownedWhen, readOwned, readOwnedResource } from "./owned";

type Unlisten = () => void;

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

export async function waitForWarmCache(
  epoch = graphEpoch(),
  deps: WarmCacheWaitDeps = defaultDeps
): Promise<boolean> {
  if (epoch !== deps.currentEpoch()) return false;

  let done = false;
  let unlisten: Unlisten | null = null;

  const finish = (ready: boolean, resolve: (ready: boolean) => void) => {
    if (done) return;
    done = true;
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
    resolve(ready && epoch === deps.currentEpoch());
  };

  return new Promise<boolean>((resolve) => {
    const owner = ownedWhen(() => !done && epoch === deps.currentEpoch());
    readOwnedResource(owner, deps.listenWarmCacheDone(() => finish(true, resolve)), (u) => u())
      .then((installed) => {
        if (installed.kind === "stale") { finish(false, resolve); return; }
        unlisten = installed.value;
        // Subscribe first, then probe the command so small graphs cannot lose the
        // event/command race. During this warm window block-ref badges stay
        // absent/zero; this does not block first paint.
        void readOwned(owner, deps.warmDone())
          .then((ready) => {
            if (ready.kind === "current" && ready.value) finish(true, resolve);
          })
          .catch(() => {
            // Keep waiting for the event; a transient IPC failure must not spin.
          });
      })
      .catch(() => {
        void readOwned(owner, deps.warmDone())
          .then((ready) => finish(ready.kind === "current" && ready.value, resolve))
          .catch(() => finish(false, resolve));
      });
  });
}
