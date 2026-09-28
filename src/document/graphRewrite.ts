import { backend } from "../backend";
import { graphOwner, writeOwned } from "../owned";
import type { PageTarget } from "../router";
import { endEdit } from "../editorController";
import { flushAll } from "./save/engine";
import { resetStore } from "./workingSet";
import { graphRewriteFrozen, tryFreezeGraphRewrite } from "./graphRewriteState";
import { pushToast } from "../toasts";
import type { RenameDone } from "../types";

let refreshRenamedNavigation: ((from: string, to: string, target?: PageTarget) => void) | null = null;

/** The app supplies route/index effects; the document intent owns the flush,
 * edit freeze, disk rewrite and working-set reset. */
export function installRenameRefreshHandler(handler: (from: string, to: string, target?: PageTarget) => void): void {
  refreshRenamedNavigation = handler;
}

/** What `renamePageOnDisk` did. The backend's answer when it ran; otherwise
 *  `busy` (another graph rewrite is in progress), `unflushed` (pending edits
 *  could not be saved) or `stale` (graph ownership retired first), all with
 *  nothing written; or `uncertain`: ownership retired after the backend call
 *  was made, so the rename may have committed and disk must be checked. */
export type DiskRename = RenameDone | "busy" | "unflushed" | "stale" | "uncertain";

/** Freeze user edits, flush current pages, then ask the backend to rename a
 * page and rewrite references across the graph (merging into `mergeInto`, the
 * confirmed owner of `to`, when given). Backend errors reject. After a rename
 * or merge, reset the working set and refresh navigation; `unchanged` touches
 * neither. Cost grows with graph pages and references. A backend failure can
 * require inspecting disk before retrying. */
export async function renamePageOnDisk(from: string, to: string, target?: PageTarget, mergeInto?: string): Promise<DiskRename> {
  if (graphRewriteFrozen()) return "busy";
  // Blur is synchronous: commit the current editor buffer before closing the
  // write gate, with no await or input event between the two steps.
  if (typeof document !== "undefined" && document.activeElement instanceof HTMLElement)
    document.activeElement.blur();
  const release = tryFreezeGraphRewrite();
  if (!release) return "busy";
  const owner = graphOwner();
  try {
    // Delayed intents now fail pageWritable even if they started before this.
    endEdit("graph-switch");
    if (!(await flushAll())) return "unflushed";
    if (!owner()) return "stale";
    let result;
    try {
      result = await writeOwned(owner, mergeInto
        ? backend().renamePage(from, to, "rename-page", target?.path, mergeInto)
        : target?.path
          ? backend().renamePage(from, to, "rename-page", target.path)
          : backend().renamePage(from, to, "rename-page"));
    } catch (error) {
      if (!owner()) pushToast(`Rename failed: ${String(error)}`, "error");
      throw error;
    }
    if (result.kind === "stale") return "uncertain";
    if (result.value === "unchanged") return "unchanged";
    resetStore();
    refreshRenamedNavigation?.(from, to, target);
    return result.value;
  } finally {
    release();
  }
}
