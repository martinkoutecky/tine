// Concord conflict queue (og family 8): which pages need the user's judgement.
//
// The queue is DERIVED, never persisted: the backend derives it from disk
// (sync-tool copies paired with their winner, marker-bearing pages) with one
// walk at graph open, then re-derives only the files each change touched and
// announces `conflicts-changed` when the answer moved. It survives a restart
// by being recomputed and nothing is written into the graph to remember it.
// The listings and the queue are one answer, held in one signal, so a banner
// can never point at an object the queue does not have. `refreshSyncConflicts`
// (ui.ts) is the only refresher.
import { createSignal } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";
import type { ConflictInventory, ConflictObject, SyncConflict } from "./types";

const EMPTY: ConflictInventory = { sync_conflicts: [], vcs_markers: [], queue: [] };

export const [conflictInventory, setConflictInventory] = createSignal<ConflictInventory>(EMPTY);

// Only the newest refresh episode may publish: an inventory walk begun before a
// guarded Apply can otherwise finish after it and resurrect the settled object.
let generation = 0;
/** Start a refresh episode; publish its answer only while it is still current. */
export function beginConflictRefresh(): number {
  return ++generation;
}
export function conflictRefreshCurrent(episode: number): boolean {
  return episode === generation;
}

// I-20: the listings name graph-relative paths; a graph switch empties them and
// drops any answer still in flight from the old graph.
clearOnBindingInvalidated(() => {
  ++generation;
  setConflictInventory(EMPTY);
});

export const conflictQueue = (): ConflictObject[] => conflictInventory().queue;
export const syncConflicts = (): SyncConflict[] => conflictInventory().sync_conflicts;
export function setSyncConflicts(sync_conflicts: SyncConflict[]): void {
  setConflictInventory((inventory) => ({ ...inventory, sync_conflicts }));
}

/** The queued conflict for the page file at `path`, if any. */
export function conflictForPage(path: string | undefined): ConflictObject | undefined {
  return path ? conflictQueue().find((conflict) => conflict.page_path === path) : undefined;
}

/** Retire one object a guarded resolve just proved gone, without waiting for a
 *  graph walk; the follow-up refresh reconciles anything else. */
export function settleArtifactConflict(id: string): void {
  const settled = conflictQueue().find((conflict) => conflict.id === id);
  if (!settled) return;
  ++generation;
  const copy = settled.sides.find((side) => side.role === "theirs")?.path;
  setConflictInventory((inventory) => ({
    sync_conflicts: settled.source === "sync-copy"
      ? inventory.sync_conflicts.filter((c) => c.path !== copy)
      : inventory.sync_conflicts,
    vcs_markers: settled.source === "vcs-markers"
      ? inventory.vcs_markers.filter((m) => m.path !== settled.page_path)
      : inventory.vcs_markers,
    queue: inventory.queue.filter((conflict) => conflict.id !== id),
  }));
}
