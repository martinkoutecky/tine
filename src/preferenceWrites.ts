/** Device preference writes. `writePreference` updates one signal and serializes
 * writes for it; failure restores the last confirmed value and shows one error.
 * `preferenceRevision` and `preferenceReadCurrent` guard startup reads against
 * later user writes; `seedPreference` records a completed startup read. Each
 * operation is O(1) plus the backend write. Callers observe the signal and toast;
 * they do not manage write ordering or rollback state. */
import { pushToast } from "./toasts";

type State<T> = { committed: T; revision: number; pending: number; queue: Promise<void> };
const states = new WeakMap<Function, State<unknown>>();

/** Apply a device preference immediately, serialize its writes, and restore the
 * last confirmed value if the latest write fails. Each failed write is reported. */
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

/** Capture before a startup read; apply its result only if no write overtook it. */
export function preferenceRevision<T>(read: () => T): number {
  return (states.get(read) as State<T> | undefined)?.revision ?? 0;
}

export function preferenceReadCurrent<T>(read: () => T, revision: number): boolean {
  const state = states.get(read) as State<T> | undefined;
  return (state?.revision ?? 0) === revision && (state?.pending ?? 0) === 0;
}

/** Call after a startup read replaces the signal's initial value. */
export function seedPreference<T>(read: () => T): void {
  const state = states.get(read) as State<T> | undefined;
  if (state && state.pending === 0) state.committed = read();
}
