import { backend, type GraphChange } from "../backend";
import { captureBinding } from "../binding";
import { graphOwner, readOwned } from "../owned";
import { bumpDataRev, bumpPageInventoryRev } from "../graphSession";
import { toLoadablePage } from "./convert";
import { doc, feedNames, pageByName } from "./model";
import { markConflict } from "./save/engine";
import { deferExternalReload, installDeferredReloadReplay } from "./deferredReload";
import { rekeyPageIdentityByPath, reloadDisposition, reloadPageIfStillSafe, restoreTodayJournalInFeed } from "./workingSet";

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

let captureExternalChangeUi: (() => ExternalChangeUi) | null = null;
export function installExternalChangeUiHandler(capture: () => ExternalChangeUi): void {
  captureExternalChangeUi = capture;
}

/** Apply an already-observed watcher change to frontend graph revisions and
 * loaded pages. Reload a safe loaded page, mark an edited page conflicted, or
 * notify route/feed UI about a removal. The watcher/backend has already
 * updated disk and its cache; this function does not persist the change.
 * Page reads may reject. Cost follows the affected page and current UI state. */
export async function applyGraphChange(c: GraphChange): Promise<void> {
  const binding = captureBinding();
  const owner = graphOwner();
  if (c.binding_generation !== undefined && c.binding_generation !== binding.backendGeneration) return;
  // The watcher has already updated the backend graph cache. Invalidate even
  // when this page is outside the bounded frontend working set.
  bumpDataRev();
  if (c.created || c.removed) bumpPageInventoryRev();
  const ui = captureExternalChangeUi?.();
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
      markConflict(currentName, { kind: "disk-changed" }, revision);
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
      restoreTodayJournalInFeed();
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
