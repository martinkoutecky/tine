// ADR 0078 §3, slice 3: on Android and iOS, git follows the app's visibility
// edges (the session-activity edges, GH #426). Going to the background, the
// pages are flushed first, so the backend's background commit includes the
// last edit; then the graphs' git threads commit (at most once per interval)
// and, when they sync automatically, push. Coming back, automatic sync runs a
// cycle. Desktop sends nothing: it commits on the cadence and at quit.
import { backend } from "./backend";
import { dbg } from "./debug";
import { ownedWhen, writeOwned } from "./owned";

/** Tell the graphs' git threads; the answer carries nothing to land. */
function tellBackend(foreground: boolean): Promise<unknown> {
  return writeOwned(ownedWhen(), backend().gitAppLifecycle(foreground));
}

// The newest edge, and the tells in the order they are sent. The background
// edge tells only after its flush, so a foreground edge can overtake it; the
// overtaken edge is then stale and never sent (review F4): a late "away"
// would leave the git threads waiting for a foreground that already came.
let latest = 0;
let told: Promise<unknown> = Promise.resolve();

/** One visibility edge. `flushAll` publishes every page's input and asset
 *  writes (the background flush, which reports its own failure). Never
 *  rejects: a failed flush still commits what was saved, and a refused call
 *  waits for the next edge or the cadence. An edge a newer one overtook is
 *  dropped. */
export async function gitLifecycleEdge(
  foreground: boolean,
  flushAll: () => Promise<boolean>,
  tell: (foreground: boolean) => Promise<unknown> = tellBackend,
): Promise<void> {
  const edge = ++latest;
  if (!foreground) {
    await flushAll().then(
      () => undefined,
      () => dbg("git background edge: page flush failed"),
    );
  }
  const sent = told.then(() => {
    if (edge !== latest) return dbg("git lifecycle edge overtaken");
    return tell(foreground).then(
      () => undefined,
      () => dbg("git lifecycle edge unrecorded"),
    );
  });
  told = sent;
  await sent;
}
