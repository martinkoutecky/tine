import { backend, type GraphChange } from "../backend";
import { captureBinding, stillBound } from "../binding";
import { conflictPolicyAlwaysAsk, holdExternalChange, installHeldExternalChangeApplier } from "../conflictPolicy";
import { pushToast } from "../toasts";
import { graphOwner, readOwned } from "../owned";
import { bumpDataRev, bumpPageInventoryRev } from "../graphSession";
import { toLoadablePage } from "./convert";
import { doc, feedNames, pageByName } from "./model";
import { applyObservedDivergence, isConflicted } from "./save/engine";
import { deferExternalReload, installDeferredReloadReplay } from "./deferredReload";
import { rekeyPageIdentityByPath, reloadDisposition, reloadPageIfStillSafe, reportPageLoadRefusal, restoreTodayJournalInFeed } from "./workingSet";

/** Route and feed actions belong to the app; the document module owns the
 * decision to call them. The snapshot keeps one watcher event on one UI view. */
export interface ExternalChangeUi {
  pageOpen(name: string): boolean;
  journalsOpen: boolean;
  leaveRemovedPage(name: string): void;
  restartJournalFeed(): void;
}

// A change declined below is recorded for replay; the replay re-enters
// `applyGraphChange`, so the disposition is re-decided with whatever state holds
// then (deferredReload.ts; GH #337).
installDeferredReloadReplay({
  ready: (page) => reloadDisposition(page) === "reload",
  run: (change) => void applyGraphChange(change),
});

// "Always ask" (conflictPolicy.ts): Reload from disk re-enters this handler with
// the policy bypassed for that one change.
installHeldExternalChangeApplier((change) => void applyGraphChange(change, true));

let captureExternalChangeUi: (() => ExternalChangeUi) | null = null;
export function installExternalChangeUiHandler(capture: () => ExternalChangeUi): void {
  captureExternalChangeUi = capture;
}

/** Apply an already-observed watcher change to frontend graph revisions and
 * loaded pages. Reload a safe loaded page, mark an edited page conflicted, or
 * notify route/feed UI about a removal. The watcher/backend has already
 * updated disk and its cache; this function does not persist the change.
 * Page reads may reject. Cost follows the affected page and current UI state. */
export async function applyGraphChange(c: GraphChange, bypassPolicy = false): Promise<void> {
  const binding = captureBinding();
  if (c.binding_generation !== undefined && c.binding_generation !== binding.backendGeneration) return;
  // The watcher has already updated the backend graph cache. Invalidate even
  // when this page is outside the bounded frontend working set.
  bumpDataRev();
  if (c.created || c.removed) bumpPageInventoryRev();
  await applyObservedChange(c, captureExternalChangeUi?.(), bypassPolicy);
}

/** One checkout-sized watcher batch (`graph-changed-bulk`, over 32 pages;
 *  master 1229f32fb): revisions move once, only pages something loads, shows
 *  or holds are applied (each through the same per-page decision), the journal
 *  feed restarts at most once, and one summary toast replaces per-page work.
 *  Cost O(changes) plus one page read per loaded or shown page. */
export async function applyGraphChangesBulk(bulk: { changes: GraphChange[]; binding_generation?: number }): Promise<void> {
  const binding = captureBinding();
  if (bulk.binding_generation !== undefined && bulk.binding_generation !== binding.backendGeneration) return;
  const changes = bulk.changes;
  if (!changes.length) return;
  bumpDataRev();
  if (changes.some((c) => c.created || c.removed)) bumpPageInventoryRev();
  const ui = captureExternalChangeUi?.();
  let restart = false, conflicts = 0;
  const batchUi = ui && { ...ui, restartJournalFeed: () => { restart = true; } };
  for (const c of changes) {
    if (!stillBound(binding)) return;
    const name = (c.path && doc.pages.find((page) => page.id === c.path)?.name) || c.name;
    // A page nothing loads or shows is refetched on navigation anyway.
    if (!c.removed && !ui?.pageOpen(c.name) && !pageByName(name) && reloadDisposition(name) === "reload") continue;
    await applyObservedChange(c, batchUi, false);
    if (isConflicted(name)) conflicts++;
  }
  if (!stillBound(binding)) return;
  if (restart || (ui?.journalsOpen && changes.some((c) => c.kind === "journal"))) ui?.restartJournalFeed();
  pushToast(`${changes.length} pages updated externally${conflicts ? ` · ${conflicts} conflict${conflicts === 1 ? "" : "s"} to review` : ""}`, "info");
}

async function applyObservedChange(c: GraphChange, ui: ExternalChangeUi | undefined, bypassPolicy: boolean): Promise<void> {
  const owner = graphOwner();
  const restartJournalFeed = () => {
    if (c.kind === "journal") ui?.restartJournalFeed();
  };
  const loadedName = c.path ? doc.pages.find((page) => page.id === c.path)?.name : undefined;
  const currentName = loadedName ?? c.name;
  const disp = reloadDisposition(currentName);
  const markObservedConflict = async () => {
    const id = pageByName(currentName)?.id;
    let revision: string | null | undefined;
    try {
      if (c.removed) revision = null;
      else {
        const result = await readOwned(owner, id
          ? backend().getPageByPath(id)
          : backend().getPage(currentName, c.kind));
        if (result.kind === "stale") return;
        revision = result.value?.rev ?? null;
      }
    } catch {
      // Without a fresh observation, the old load revision remains a
      // conservative guard: Keep mine cannot clobber changed bytes.
    }
    if (owner() && pageByName(currentName)?.id === id && reloadDisposition(currentName) === "conflict")
      applyObservedDivergence(currentName, revision);
  };
  if (c.removed) {
    if (disp === "conflict") await markObservedConflict();
    if (disp === "conflict" || disp === "skip") {
      if (disp === "skip") deferExternalReload(currentName, c);
      restartJournalFeed();
      return;
    }
    ui?.leaveRemovedPage(c.name);
    if (c.kind === "journal" && ui?.journalsOpen) {
      // Another file holding today's name keeps it out of the feed (og J1);
      // the restart below retries once that holder is replaceable.
      const refused = restoreTodayJournalInFeed();
      if (refused) reportPageLoadRefusal(refused);
      restartJournalFeed();
    }
    return;
  }

  if (disp === "conflict") await markObservedConflict();
  if (disp === "conflict" || disp === "skip") {
    if (disp === "skip") deferExternalReload(currentName, c);
    restartJournalFeed();
    return;
  }
  // "Always ask": reached only after the conflict/skip branches, so it turns
  // the one SILENT case (a loaded, clean page) into an asked one and changes
  // nothing that already asked or deferred.
  if (!bypassPolicy && conflictPolicyAlwaysAsk() && pageByName(currentName)) {
    holdExternalChange(currentName, c);
    return;
  }
  if (c.path && loadedName && loadedName !== c.name) {
    const result = await readOwned(owner, backend().getPageByPath(c.path));
    if (result.kind === "stale" || !result.value || result.value.id !== c.path) return;
    if (!rekeyPageIdentityByPath(c.path, result.value.name, result.value.rev ?? null)) return;
    reloadPageIfStillSafe(result.value.name, toLoadablePage(result.value, result.value.name));
    restartJournalFeed();
    return;
  }
  if (ui?.pageOpen(c.name)) {
    const result = await readOwned(owner, backend().getPage(c.name, c.kind));
    if (result.kind === "stale") return;
    // A decline here (the page turned busy during the read) is the same dropped
    // reload as "skip": defer, don't drop.
    if (result.value && !reloadPageIfStillSafe(c.name, toLoadablePage(result.value, c.name))) deferExternalReload(currentName, c);
    restartJournalFeed();
    return;
  }
  if (c.kind === "journal" && ui?.journalsOpen) {
    if (pageByName(c.name)) {
      const result = await readOwned(owner, backend().getPage(c.name, c.kind));
      if (result.kind === "stale") return;
      if (result.value && !reloadPageIfStillSafe(c.name, result.value)) deferExternalReload(currentName, c);
    }
    // The feed owner gates dirty/save/conflict/move state and records a pending
    // restart when unsafe, so this watcher event is not lost.
    restartJournalFeed();
    return;
  }
  if (pageByName(c.name) && !feedNames().includes(c.name)) {
    const result = await readOwned(owner, backend().getPage(c.name, c.kind));
    if (result.kind === "stale") return;
    if (result.value && !reloadPageIfStillSafe(c.name, result.value)) deferExternalReload(currentName, c);
  }
}
