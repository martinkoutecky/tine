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
import { captureBinding, clearOnBindingInvalidated, stillBound, type Binding } from "./binding";

const KEY = "concord_always_ask";
const [alwaysAsk, setAlwaysAsk] = createSignal(false);

/** Reactive: hold external changes for review instead of applying them. */
export const conflictPolicyAlwaysAsk = alwaysAsk;

export function setConflictPolicyAlwaysAsk(on: boolean): void {
  setAlwaysAsk(on);
  if (!on) clearHeldExternalChanges();
  void backend().setAppBool(KEY, on).catch(() => {});
}

/** Load the persisted preference. Default off (silent freshness). */
export async function initConflictPolicy(): Promise<void> {
  try { setAlwaysAsk(await backend().getAppBool(KEY, false)); } catch { /* default off */ }
}

interface Held { change: GraphChange; binding: Binding }
const [held, setHeld] = createSignal<Record<string, Held>>({});

/** Whether page `name` has an external change waiting for its owner. */
export function heldExternalChangeFor(name: string | undefined): boolean {
  const pending = name ? held()[name] : undefined;
  return !!pending && stillBound(pending.binding);
}

/** Record a change the policy asks about; the latest observation wins (the
 *  apply refetches the page, so only the newest change matters). */
export function holdExternalChange(name: string, change: GraphChange): void {
  setHeld((current) => ({ ...current, [name]: { change, binding: captureBinding() } }));
}

function take(name: string): Held | undefined {
  const pending = held()[name];
  if (pending) setHeld(({ [name]: _, ...rest }) => rest);
  return pending && stillBound(pending.binding) ? pending : undefined;
}

export function clearHeldExternalChanges(): void { setHeld({}); }
clearOnBindingInvalidated(clearHeldExternalChanges);

let applier: ((change: GraphChange) => void) | null = null;
/** Installed once by the watcher handler: the bar re-enters the SAME
 *  external-change path, never a private reload. */
export function installHeldExternalChangeApplier(handler: (change: GraphChange) => void): void { applier = handler; }

/** "Reload from disk": re-dispatch with the policy bypassed for this change;
 *  every other gate (disposition, editing, deferred replay) still applies. */
export function applyHeldExternalChange(name: string): void {
  const pending = take(name);
  if (pending) applier?.(pending.change);
}

/** "Keep mine": drop the record; nothing is written. */
export function dismissHeldExternalChange(name: string): void { take(name); }
