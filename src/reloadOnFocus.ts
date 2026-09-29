// Family 10 — reload on focus (Concord L0; master d56219d73, b3d64addee39).
//
// The watcher is the primary freshness path and stays so. Some filesystems and
// sync clients deliver no event at all (network mounts, a client writing
// through a path the kernel does not report, an app the OS suspended while the
// user was elsewhere), and then a page can sit stale indefinitely. The one
// signal always available is the user coming back to the window. Neither step
// below is a new freshness path:
//  1. `replayDeferredExternalReloads()` replays reloads deferred mid-edit
//     (deferredReload.ts) whose page is replaceable now;
//  2. `rescanGraphNow()` asks the BACKEND watcher for one full stat diff.
//     What it finds is emitted as ordinary `graph-changed` events, so the
//     reload disposition and the deferred replay apply as for a live event.
// Until the rescan's events are applied, the freshness barrier holds new
// edits. Throttled: a focus is a gesture users make constantly, and a rescan
// costs one stat per graph-text file. Coalesced: a focus during a rescan of the
// same graph joins it.
import { backend } from "./backend";
import { captureBinding, stillBound, type Binding } from "./binding";
import { applyGraphChangesBulk, replayDeferredExternalReloads } from "./document";
import { beginFreshnessBarrier, endFreshnessBarrier, installFreshnessInputGate } from "./freshnessBarrier";
import { ownedWhen, readOwnedResource, type Owned } from "./owned";
import { isPublishedExport } from "./publishedBackend";
import { pushToast } from "./toasts";

/** Minimum spacing between focus-driven rescans; below it, a return to the
 *  window is answered by the in-memory replay alone. */
export const FOCUS_RESCAN_THROTTLE_MS = 1500;
const COMPLETION_TIMEOUT_MS = 30_000;

let lastRescan = 0;
let active: { refresh: Promise<void>; binding: Binding } | null = null;
let stateBinding: Binding | null = null;
let completed = 0;
let listener: Promise<unknown> | null = null;
const waiters = new Map<number, { resolve: () => void; reject: (error: Error) => void }>();
const applications = new Set<Promise<unknown>>();

class StaleFocusRefresh extends Error {}

/** A graph switch retires the throttle and every pending completion. */
function retireChangedBinding(): boolean {
  if (stateBinding && stillBound(stateBinding)) return false;
  stateBinding = captureBinding();
  lastRescan = 0;
  for (const waiter of waiters.values()) waiter.reject(new StaleFocusRefresh());
  waiters.clear();
  return true;
}

/** Track the async application of one native graph-change event: the rescan
 *  completion follows the events, but their handlers may still await reads. */
export function trackGraphChangeApplication(work: Promise<unknown>): void {
  applications.add(work);
  // Tracking never consumes a failure: a rejection is re-raised exactly as the
  // untracked `void applyGraphChange(c)` raised it before.
  const untrack = () => { applications.delete(work); };
  void work.then(untrack, (error: unknown) => { untrack(); throw error; });
}

function ensureCompletionListener(subscribe: (cb: (sequence: number) => void) => Promise<() => void>): Promise<unknown> {
  listener ??= subscribe((sequence) => {
    completed = Math.max(completed, sequence);
    for (const [target, waiter] of waiters) if (target <= completed) { waiters.delete(target); waiter.resolve(); }
  }).catch((error) => { listener = null; throw error; });
  return listener;
}

function waitForCompletion(sequence: number): Promise<void> {
  if (sequence <= completed) return Promise.resolve();
  return new Promise<void>((resolve, reject) => {
    const waiter = { resolve, reject };
    waiters.set(sequence, waiter);
    setTimeout(() => {
      if (waiters.get(sequence) !== waiter) return;
      waiters.delete(sequence);
      reject(new Error(`watcher rescan ${sequence} did not complete`));
    }, COMPLETION_TIMEOUT_MS);
  });
}

function releaseActive(refresh: Promise<void>): void {
  if (active?.refresh === refresh) active = null;
}

/** Exported for tests; `installReloadOnFocus` wires it to focus/visibility. */
export function refreshOnReturnToWindow(now = Date.now()): Promise<void> {
  // A published export is an immutable snapshot with no watcher behind it.
  if (isPublishedExport()) return Promise.resolve();
  replayDeferredExternalReloads();
  const changed = retireChangedBinding();
  if (active) {
    if (!changed && stillBound(active.binding)) return active.refresh;
    return active.refresh.then(() => refreshOnReturnToWindow(now));
  }
  const api = backend();
  if (!api.rescanGraphNow || !api.onGraphRescanComplete || now - lastRescan < FOCUS_RESCAN_THROTTLE_MS) return Promise.resolve();
  lastRescan = now;
  const binding = stateBinding!;
  const current = () => { if (!stillBound(binding)) throw new StaleFocusRefresh(); };
  beginFreshnessBarrier();
  let refresh!: Promise<void>;
  refresh = (async () => {
    try {
      await ensureCompletionListener((cb) => api.onGraphRescanComplete!(cb));
      current();
      const sequence = await api.rescanGraphNow!();
      current();
      await waitForCompletion(sequence);
      while (applications.size) {
        await Promise.allSettled([...applications]);
        current();
      }
      replayDeferredExternalReloads();
    } catch (error) {
      if (error instanceof StaleFocusRefresh) return;
      // The watcher stays primary; a failed fallback must release the gate.
      pushToast(`Tine couldn't finish checking for external changes. Editing is available, but reopen the page before relying on it being current. (${String(error)})`, "error");
    } finally {
      endFreshnessBarrier();
      releaseActive(refresh);
    }
  })();
  active = { refresh, binding };
  return refresh;
}

/** Reset the time throttle only (tests). */
export function resetFocusRescanThrottle(): void { lastRescan = 0; }

let installed = false;
export function installReloadOnFocus(): void {
  if (installed || typeof window === "undefined" || isPublishedExport()) return;
  installed = true;
  installFreshnessInputGate();
  window.addEventListener("focus", () => void refreshOnReturnToWindow());
  document.addEventListener("visibilitychange", () => { if (!document.hidden) void refreshOnReturnToWindow(); });
}

/** Subscribe the window to the watcher's checkout-sized batches and to a
 *  refused/restored OS watch. A refusal (inotify's per-user watch limit, a
 *  network mount or filesystem without notifications) is said out loud: the
 *  backend polls every 3 seconds meanwhile, so the graph is never silently
 *  stale (I-9). Returns the unsubscribe. */
export function subscribeWatcherFreshness(): () => void {
  let alive = true;
  const owner = ownedWhen(() => alive);
  const unsubs: (() => void)[] = [];
  const keep = (result: Owned<() => void>) => { if (result.kind === "current") unsubs.push(result.value); };
  const api = backend();
  if (api.onGraphChangedBulk) void readOwnedResource(owner, api.onGraphChangedBulk((bulk) => trackGraphChangeApplication(applyGraphChangesBulk(bulk))), (u) => u()).then(keep);
  if (api.onGraphWatchStatus) void readOwnedResource(owner, api.onGraphWatchStatus((status) => {
    if (status.binding_generation !== undefined && status.binding_generation !== captureBinding().backendGeneration) return;
    if (status.refused) pushToast(`Live file notifications are unavailable for this graph (${status.message}). Tine checks for external changes every 3 seconds instead.`, "warn", { sticky: true });
    else pushToast("Live file notifications are back for this graph.", "info");
  }), (u) => u()).then(keep);
  return () => { alive = false; for (const unsub of unsubs) unsub(); };
}
