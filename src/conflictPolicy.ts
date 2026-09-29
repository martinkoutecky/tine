// Concord P5 "always ask" (master; family 10). Tine's only silent adoption of
// external bytes is freshness: a page you have loaded, with nothing unsaved,
// changes on disk and Tine shows the new content (what VS Code and IntelliJ
// do). With ALWAYS ASK on, that one case is HELD instead: the page keeps what
// you were reading and offers Reload from disk / Keep mine.
//
// Nothing that already asks stops asking: a dirty page still takes the
// conflict path, a page being edited still defers (deferredReload.ts). Holding
// is frontend-only and writes nothing: the backend cache already has the new
// bytes, so after "Keep mine" the next save meets the base-revision guard and
// raises the ordinary conflict bar.
import { createSignal } from "solid-js";
import { backend, type GraphChange } from "./backend";
import { graphScopedSignal } from "./binding";
import { advanceRevision, currentRevision, readOwned, revisionOwner, writeOwned } from "./owned";
import { pushToast } from "./toasts";

const KEY = "concord_always_ask";
const [alwaysAsk, setAlwaysAsk] = createSignal(false);
const preferenceKey = {};

/** Reactive: hold external changes for review instead of applying them. */
export const conflictPolicyAlwaysAsk = alwaysAsk;

/** Device preference; a later choice wins a delayed load, a failed write is
 *  reported (the in-session choice still applies). */
export function setConflictPolicyAlwaysAsk(on: boolean): void {
  const revision = advanceRevision(preferenceKey);
  setAlwaysAsk(on);
  if (!on) clearHeldExternalChanges();
  void writeOwned(revisionOwner(preferenceKey, revision), backend().setAppBool(KEY, on))
    .catch((error) => pushToast(`Could not remember “Always ask”: ${String(error)}`, "error"));
}

/** Load the persisted preference. Default off (silent freshness); a failed
 *  read keeps the default and says so. */
export async function initConflictPolicy(): Promise<void> {
  const owner = revisionOwner(preferenceKey, currentRevision(preferenceKey));
  try {
    const loaded = await readOwned(owner, backend().getAppBool(KEY, false));
    if (loaded.kind === "current") setAlwaysAsk(loaded.value);
  } catch (error) {
    pushToast(`Could not load “Always ask”: ${String(error)}`, "error");
  }
}

// Page name -> the newest held change. Graph-scoped: a graph switch drops it,
// so a held change never applies in another graph.
const [heldChanges, setHeldChanges] = graphScopedSignal<Record<string, GraphChange>>();

/** Whether page `name` has an external change waiting for its owner. */
export function heldExternalChangeFor(name: string | undefined): boolean {
  return !!name && !!heldChanges()?.[name];
}

/** Record a change the policy asks about; the latest observation wins (the
 *  apply refetches the page, so only the newest change matters). */
export function holdExternalChange(name: string, change: GraphChange): void {
  setHeldChanges({ ...(heldChanges() ?? {}), [name]: change });
}

function take(name: string): GraphChange | undefined {
  const current = heldChanges() ?? {};
  const pending = current[name];
  if (pending) { const { [name]: _, ...rest } = current; setHeldChanges(rest); }
  return pending;
}

export function clearHeldExternalChanges(): void { setHeldChanges(null); }

let applier: ((change: GraphChange) => void) | null = null;
/** Installed once by the watcher handler: the bar re-enters the SAME
 *  external-change path, never a private reload. */
export function installHeldExternalChangeApplier(handler: (change: GraphChange) => void): void { applier = handler; }

/** "Reload from disk": re-dispatch with the policy bypassed for this change;
 *  every other gate (disposition, editing, deferred replay) still applies. */
export function applyHeldExternalChange(name: string): void {
  const pending = take(name);
  if (pending) applier?.(pending);
}

/** "Keep mine": drop the record; nothing is written. */
export function dismissHeldExternalChange(name: string): void { take(name); }
