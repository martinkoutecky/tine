import { backend } from "./backend";
import { graphEpoch } from "./ui";

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
    deps
      .listenWarmCacheDone(() => finish(true, resolve))
      .then((u) => {
        if (done) {
          u();
          return;
        }
        unlisten = u;
        // Subscribe first, then probe the command so small graphs cannot lose the
        // event/command race. During this warm window aliases render as page names
        // and block-ref badges stay absent/zero; neither path blocks first paint.
        void deps
          .warmDone()
          .then((ready) => {
            if (ready) finish(true, resolve);
          })
          .catch(() => {
            // Keep waiting for the event; a transient IPC failure must not spin.
          });
      })
      .catch(() => {
        void deps
          .warmDone()
          .then((ready) => finish(ready, resolve))
          .catch(() => finish(false, resolve));
      });
  });
}
