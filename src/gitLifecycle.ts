// ADR 0078 §3, slice 3: on Android and iOS, git follows the app's visibility
// edges (the session-activity edges, GH #426). Going to the background, the
// pages are flushed first, so the backend's background commit includes the
// last edit; then the graphs' git threads commit and, when they sync
// automatically, push. Coming back, they commit and sync. Desktop sends
// nothing: it commits on the cadence and at quit.
import { backend } from "./backend";
import { ownedWhen, writeOwned } from "./owned";

/** Tell the graphs' git threads; the answer carries nothing to land. */
function tellBackend(foreground: boolean): Promise<unknown> {
  return writeOwned(ownedWhen(), backend().gitAppLifecycle(foreground));
}

/** One visibility edge. `flushAll` publishes every page's input and asset
 *  writes (the background flush). Best effort: never rejects; a failed flush
 *  still commits what was saved, and a refused call waits for the next edge. */
export async function gitLifecycleEdge(
  foreground: boolean,
  flushAll: () => Promise<boolean>,
  tell: (foreground: boolean) => Promise<unknown> = tellBackend,
): Promise<void> {
  if (!foreground) await flushAll().catch(() => false);
  await tell(foreground).catch(() => undefined);
}
