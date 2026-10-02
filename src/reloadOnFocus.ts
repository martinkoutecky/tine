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
import { graphTransitioning } from "./ui";

/** Minimum spacing between focus-driven rescans; below it, a return to the
 *  window is answered by the in-memory replay alone. */
export const FOCUS_RESCAN_THROTTLE_MS = 1500;
const COMPLETION_TIMEOUT_MS = 30_000;

let lastRescan = 0;
/** When the last rescan (focus or Settings) completed and its events were applied. */
let lastFinishedAt: number | null = null;
let finishedCount = 0;
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

/** Whether this window has a graph to rescan. Before the launch/switch load
 *  has published its binding (Welcome screen, or load_graph still running or
 *  its answer not yet received) the backend refuses the rescan with
 *  `no graph loaded for window …` / `missing-graph-binding`. That is not a
 *  failure: the load that installs the binding reads the disk itself, and its
 *  watcher subscription starts at the revision it read. So the fallback is
 *  sequenced after the binding, never reported against it (OG-TOAST T1). */
function graphReadyForRescan(): boolean {
  return captureBinding().backendGeneration !== 0 && !graphTransitioning();
}

function releaseActive(refresh: Promise<void>): void {
  if (active?.refresh === refresh) active = null;
}

/** Exported for tests; `installReloadOnFocus` wires it to focus/visibility. */
export function refreshOnReturnToWindow(now = Date.now(), force = false, rebuild = false): Promise<void> {
  // A published export is an immutable snapshot with no watcher behind it.
  if (isPublishedExport()) return Promise.resolve();
  replayDeferredExternalReloads();
  if (!graphReadyForRescan()) return Promise.resolve();
  const changed = retireChangedBinding();
  if (active) {
    // A forced (Settings) rescan must itself start after the click, so it waits
    // for a rescan already in flight and runs its own.
    if (!force && !changed && stillBound(active.binding)) return active.refresh;
    return active.refresh.then(() => refreshOnReturnToWindow(now, force, rebuild));
  }
  const api = backend();
  if (!api.rescanGraphNow || !api.onGraphRescanComplete || (!force && now - lastRescan < FOCUS_RESCAN_THROTTLE_MS)) return Promise.resolve();
  lastRescan = now;
  const binding = stateBinding!;
  const current = () => { if (!stillBound(binding)) throw new StaleFocusRefresh(); };
  beginFreshnessBarrier();
  let refresh!: Promise<void>;
  refresh = (async () => {
    try {
      await ensureCompletionListener((cb) => api.onGraphRescanComplete!(cb));
      current();
      const sequence = await api.rescanGraphNow!(rebuild);
      current();
      await waitForCompletion(sequence);
      while (applications.size) {
        await Promise.allSettled([...applications]);
        current();
      }
      replayDeferredExternalReloads();
      lastFinishedAt = Date.now();
      finishedCount++;
    } catch (error) {
      // A refusal because the graph was switched or restored meanwhile is the
      // stale case too: the new binding's load read the disk itself.
      if (error instanceof StaleFocusRefresh || !stillBound(binding)) return;
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

/** Settings → Help & diagnostics "Rescan graph": a forced full rebuild on demand
 *  (every file re-read and re-parsed, ignoring stamps; the focus-return rescan
 *  stays the cheap stat diff), unthrottled but through the same barrier-holding
 *  path as a focus rescan, so its changes are applied before it reports.
 *  Answers when it finished (ms since the epoch), or `null` when no rescan ran (no graph loaded, published export)
 *  or it failed (the failure is already toasted by the shared path). */
export async function rescanGraphNowFromSettings(): Promise<number | null> {
  const before = finishedCount;
  await refreshOnReturnToWindow(Date.now(), true, true);
  return finishedCount !== before ? lastFinishedAt : null;
}

/** Reset the time throttle only (tests). */
export function resetFocusRescanThrottle(): void { lastRescan = 0; lastFinishedAt = null; finishedCount = 0; }

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
