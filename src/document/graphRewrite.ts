import { backend } from "../backend";
import { graphOwner, writeOwned } from "../owned";
import type { PageTarget } from "../router";
import { endEdit } from "../editorController";
import { flushAll } from "./save/engine";
import { resetStore } from "./workingSet";
import { graphRewriteFrozen, tryFreezeGraphRewrite } from "./graphRewriteState";
import { pushToast } from "../toasts";

let refreshRenamedNavigation: ((from: string, to: string, target?: PageTarget) => void) | null = null;

/** The app supplies route/index effects; the document intent owns the flush,
 * edit freeze, disk rewrite and working-set reset. */
export function installRenameRefreshHandler(handler: (from: string, to: string, target?: PageTarget) => void): void {
  refreshRenamedNavigation = handler;
}

/** Freeze user edits, flush current pages, then ask the backend to rename a
 * page and rewrite references across the graph. Returns false when already
 * frozen, flush fails or graph ownership retires; backend errors reject.
 * On success reset the working set and refresh navigation. Cost grows with
 * graph pages and references. A backend failure can require inspecting disk
 * before retrying. */
export async function renamePageOnDisk(from: string, to: string, target?: PageTarget): Promise<boolean> {
  if (graphRewriteFrozen()) return false;
  // Blur is synchronous: commit the current editor buffer before closing the
  // write gate, with no await or input event between the two steps.
  if (typeof document !== "undefined" && document.activeElement instanceof HTMLElement)
    document.activeElement.blur();
  const release = tryFreezeGraphRewrite();
  if (!release) return false;
  const owner = graphOwner();
  try {
    // Delayed intents now fail pageWritable even if they started before this.
    endEdit("graph-switch");
    if (!(await flushAll()) || !owner()) return false;
    let result;
    try {
      result = await writeOwned(owner, target?.path
        ? backend().renamePage(from, to, "rename-page", target.path)
        : backend().renamePage(from, to, "rename-page"));
    } catch (error) {
      if (!owner()) pushToast(`Rename failed: ${String(error)}`, "error");
      throw error;
    }
    if (result.kind === "stale") return false;
    resetStore();
    refreshRenamedNavigation?.(from, to, target);
    return true;
  } finally {
    release();
  }
}
