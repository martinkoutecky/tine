import { msg } from "./i18n";
/** Device preference tracking is keyed by stable read-callback identity. Reuse
 * one callback per signal for revisions, seeds, startup reads and writes; never
 * share it across signals. Reads land only for the captured revision. Writes
 * apply now and persist sequentially; failed latest writes roll back and every
 * write failure toasts. Another window's change re-reads once pending writes
 * settle. O(1) frontend work plus backend read/write latency. */
import { pushToast } from "./toasts";
import { advanceRevision, currentRevision, revisionOwner, readOwned } from "./owned";

/** `reconcile`: another window's change arrived while writes were pending, so
 * neither their outcome nor the rollback value says what the file now holds;
 * re-read once they settle (GH #151 re-review F2). */
type State<T> = { committed: T; pending: number; queue: Promise<void>; reconcile?: () => void };
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
  onFailure?: () => void,
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
      if (onFailure) onFailure();
      else pushToast(msg("preference-writes-could-not-save", { label: String(label) }), "error");
    } finally {
      if (--state.pending === 0 && state.reconcile) {
        const reload = state.reconcile;
        state.reconcile = undefined;
        reload();
      }
    }
  });
}

/** Another window persisted this preference. Retire in-flight reads and the
 * rollback of pending writes, then run `reload` (a read through this module's
 * owners) now, or once this window's pending writes settle: the last write to
 * reach the file is unknown here, so only a read after them says what it holds.
 * O(1); `reload` runs at most once per settle. */
export function preferenceChangedElsewhere<T>(read: () => T, reload: () => void): void {
  advanceRevision(read);
  const state = states.get(read) as State<T> | undefined;
  if (state && state.pending > 0) state.reconcile = reload;
  else reload();
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
  }).catch(() => { if (owner()) pushToast(msg("preference-writes-could-not-load", { label: String(label) }), "error"); });
}
