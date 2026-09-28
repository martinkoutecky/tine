/** Device preference tracking is keyed by stable read-callback identity. Reuse
 * one callback per signal for revisions, seeds and writes; never share it across
 * signals. Writes apply now and persist sequentially. Failed latest writes roll
 * back and every failure toasts. O(1) frontend work plus backend write latency. */
import { pushToast } from "./toasts";

type State<T> = { committed: T; revision: number; pending: number; queue: Promise<void> };
const states = new WeakMap<Function, State<unknown>>();

/** Apply now and queue persistence by read-callback identity. The callback is
 * invoked only on first write for this key. Latest failure rolls back; every
 * failure toasts. Return does not confirm persistence. O(1) plus backend write. */
export function writePreference<T>(
  read: () => T,
  apply: (value: T) => void,
  value: T,
  persist: (value: T) => Promise<unknown>,
  label: string,
): void {
  let state = states.get(read) as State<T> | undefined;
  if (!state) {
    state = { committed: read(), revision: 0, pending: 0, queue: Promise.resolve() };
    states.set(read, state as State<unknown>);
  }
  const revision = ++state.revision;
  state.pending++;
  apply(value);
  state.queue = state.queue.then(async () => {
    try {
      await persist(value);
      state.committed = value;
    } catch {
      if (state.revision === revision) apply(state.committed);
      pushToast(`Could not save ${label}.`, "error");
    } finally {
      state.pending--;
    }
  });
}

/** Capture a stable callback's revision before a startup read, without calling
 * it. O(1). */
export function preferenceRevision<T>(read: () => T): number {
  return (states.get(read) as State<T> | undefined)?.revision ?? 0;
}

/** True when this callback has the captured revision and no pending writes.
 * Does not call read. O(1). */
export function preferenceReadCurrent<T>(read: () => T, revision: number): boolean {
  const state = states.get(read) as State<T> | undefined;
  return (state?.revision ?? 0) === revision && (state?.pending ?? 0) === 0;
}

/** Seed the committed value only for a tracked callback with no pending writes.
 * Calls read but does not persist. O(1). */
export function seedPreference<T>(read: () => T): void {
  const state = states.get(read) as State<T> | undefined;
  if (state && state.pending === 0) state.committed = read();
}
