import { backend, type GraphChange } from "../backend";
import { captureBinding } from "../binding";
import { graphOwner, readOwned } from "../owned";
import { bumpDataRev, bumpPageInventoryRev } from "../graphSession";
import { toLoadablePage } from "./convert";
import { feedNames, pageByName } from "./model";
import { markConflict } from "./save/engine";
import { reloadDisposition, reloadPageIfStillSafe, restoreTodayJournalInFeed } from "./workingSet";

/** Route and feed actions belong to the app; the document module owns the
 * decision to call them. The snapshot keeps one watcher event on one UI view. */
export interface ExternalChangeUi {
  pageOpen(name: string): boolean;
  journalsOpen: boolean;
  leaveRemovedPage(name: string): void;
  restartJournalFeed(): void;
}

let captureExternalChangeUi: (() => ExternalChangeUi) | null = null;
export function installExternalChangeUiHandler(capture: () => ExternalChangeUi): void {
  captureExternalChangeUi = capture;
}

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
  const disp = reloadDisposition(c.name);
  const markObservedConflict = async () => {
    const id = pageByName(c.name)?.id;
    let revision: string | null | undefined;
    try {
      if (c.removed) revision = null;
      else {
        const result = await readOwned(owner, id
          ? backend().getPageByPath(id)
          : backend().getPage(c.name, c.kind));
        if (result.kind === "stale") return;
        revision = result.value?.rev ?? null;
      }
    } catch {
      // Without a fresh observation, the old load revision remains a
      // conservative guard: Keep mine cannot clobber changed bytes.
    }
    if (owner() && pageByName(c.name)?.id === id && reloadDisposition(c.name) === "conflict")
      markConflict(c.name, { kind: "disk-changed" }, revision);
  };
  if (c.removed) {
    if (disp === "conflict") await markObservedConflict();
    if (disp === "conflict" || disp === "skip") {
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
    restartJournalFeed();
    return;
  }
  if (ui?.pageOpen(c.name)) {
    const result = await readOwned(owner, backend().getPage(c.name, c.kind));
    if (result.kind === "stale") return;
    if (result.value) reloadPageIfStillSafe(c.name, toLoadablePage(result.value, c.name));
    restartJournalFeed();
    return;
  }
  if (c.kind === "journal" && ui?.journalsOpen) {
    if (pageByName(c.name)) {
      const result = await readOwned(owner, backend().getPage(c.name, c.kind));
      if (result.kind === "stale") return;
      if (result.value) reloadPageIfStillSafe(c.name, result.value);
    }
    // The feed owner gates dirty/save/conflict/move state and records a pending
    // restart when unsafe, so this watcher event is not lost.
    restartJournalFeed();
    return;
  }
  if (pageByName(c.name) && !feedNames().includes(c.name)) {
    const result = await readOwned(owner, backend().getPage(c.name, c.kind));
    if (result.kind === "stale") return;
    if (result.value) reloadPageIfStillSafe(c.name, result.value);
  }
}
