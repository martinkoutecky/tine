// Optimistic asset writes that a destructive transition (graph switch, restore,
// app close) must wait for before it lets the bytes' references go. Shared by
// the save engine's `flushAll` and the page-host client's `settle` (step 3b),
// so both drain the same set.

const pending = new Set<Promise<boolean>>();
let started = 0;

/** Track an optimistic asset write so flushAll/app-close waits for the bytes to
 *  land before letting the process exit. The caller still owns success/failure
 *  handling for any UI/store rollback. */
export function trackAssetWrite<T>(write: Promise<T>): Promise<T> {
  started += 1;
  const tracked: Promise<boolean> = write.then(
    () => true,
    () => false
  ).finally(() => {
    pending.delete(tracked);
  });
  pending.add(tracked);
  return write;
}

/** The asset writes still running, as a snapshot. */
export function pendingAssetWrites(): Promise<boolean>[] {
  return [...pending];
}

/** How many asset writes have started: a change across a barrier means one
 *  began (and may have finished) inside it. */
export function assetWritesStarted(): number {
  return started;
}
