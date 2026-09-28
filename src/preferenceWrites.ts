/** Device preference tracking is keyed by stable read-callback identity. Reuse
 * one callback per signal for revisions, seeds, startup reads and writes; never
 * share it across signals. Reads land only for the captured revision. Writes
 * apply now and persist sequentially; failed latest writes roll back and every
 * write failure toasts. O(1) frontend work plus backend read/write latency. */
import { pushToast } from "./toasts";
import { advanceRevision, currentRevision, revisionOwner, readOwned } from "./owned";

type State<T> = { committed: T; pending: number; queue: Promise<void> };
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
    state = { committed: read(), pending: 0, queue: Promise.resolve() };
    states.set(read, state as State<unknown>);
  }
  const revision = advanceRevision(read);
  state.pending++;
  apply(value);
  state.queue = state.queue.then(async () => {
    try {
      await persist(value);
      state.committed = value;
    } catch {
      if (revisionOwner(read, revision)()) apply(state.committed);
      pushToast(`Could not save ${label}.`, "error");
    } finally {
      state.pending--;
    }
  });
}

/** Capture a stable callback's revision before a startup read, without calling
 * it. O(1). */
export function preferenceRevision<T>(read: () => T): number {
  return currentRevision(read);
}

/** True when this callback has the captured revision and no pending writes.
 * Does not call read. O(1). */
export function preferenceReadCurrent<T>(read: () => T, revision: number): boolean {
  const state = states.get(read) as State<T> | undefined;
  return revisionOwner(read, revision, () => (state?.pending ?? 0) === 0)();
}

/** Seed the committed value only for a tracked callback with no pending writes.
 * Calls read but does not persist. O(1). */
export function seedPreference<T>(read: () => T): void {
  const state = states.get(read) as State<T> | undefined;
  if (state && state.pending === 0) state.committed = read();
}

/** Load a device preference only while its callback revision and optional view
 * owner remain current. A current failure toasts; a stale completion is inert.
 * O(1) frontend work plus the supplied backend read. */
export function loadPreference<T, U>(
  read: () => T,
  apply: (value: T) => void,
  load: () => Promise<U>,
  decode: (value: U) => T,
  label: string,
  live: () => boolean = () => true,
): void {
  const revision = preferenceRevision(read);
  const owner = () => live() && preferenceReadCurrent(read, revision);
  void readOwned(owner, load()).then((result) => {
    if (result.kind === "stale") return;
    apply(decode(result.value));
    seedPreference(read);
  }).catch(() => { if (owner()) pushToast(`Could not load ${label}.`, "error"); });
}
