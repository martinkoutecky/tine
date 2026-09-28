import { backend } from "../backend";
import { graphOwner, readOwned, writeOwned } from "../owned";
import type { PageTarget } from "../router";
import { endEdit } from "../editorController";
import { conflicts, dirtyPages, flushAll, savingPages } from "./save/engine";
import { forgetPage, reloadDisposition, reloadPageIfStillSafe } from "./workingSet";
import { doc, pageByName } from "./model";
import { invalidateUndoForPage } from "./history";
import { toLoadablePage } from "./convert";
import { graphRewriteFrozen, tryFreezeGraphRewrite } from "./graphRewriteState";
import { pushToast } from "../toasts";
import { pageIdentityKey } from "../ui";
import type { RenameDone, RenameTouchedPage } from "../types";

let refreshRenamedNavigation: ((from: string, to: string, target?: PageTarget) => void) | null = null;

/** The app supplies route/index effects; the document intent owns the flush,
 * edit freeze, disk rewrite and the refresh of the pages it touched. */
export function installRenameRefreshHandler(handler: (from: string, to: string, target?: PageTarget) => void): void {
  refreshRenamedNavigation = handler;
}

/** What `renamePageOnDisk` did. The backend's outcome when it ran; otherwise
 *  `busy` (another graph rewrite is in progress), `{ unsaved }` (that page's
 *  edits could not be saved and the rename would change it: it is the renamed
 *  page or a namespace child, or its unsaved text `mentions` the old name) or
 *  `stale` (graph ownership retired first), all with nothing written; or
 *  `uncertain`: ownership retired after the backend call was made, so the
 *  rename may have committed and disk must be checked. */
export type DiskRename = RenameDone["outcome"] | "busy" | { unsaved: string; mentions: boolean } | "stale" | "uncertain";

/** Freeze user edits, save every pending edit, then ask the backend to rename
 * a page and rewrite references across the graph (merging into `mergeInto`,
 * the confirmed owner of `to`, when given). A page whose edits cannot be saved
 * blocks the rename only when the rename would change it (GH #535); any other
 * such page keeps its unsaved edits, and the backend refuses to rewrite its
 * file (that refusal rejects, naming the page). Backend errors reject. After a
 * rename or merge, drop the pages it moved under their old names, reload the
 * clean pages it rewrote, and refresh navigation; every other loaded page keeps
 * its state, unsaved edits and undo included. `unchanged` touches nothing.
 * Cost grows with graph pages and references, plus one page read per loaded
 * rewritten page. A backend failure can require inspecting disk before
 * retrying. */
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
    const prepared = await unsavedPathsFor(from);
    if (!Array.isArray(prepared)) return prepared;
    if (!owner()) return "stale";
    let result;
    try {
      result = await writeOwned(owner, backend().renamePage(from, to, "rename-page", target?.path, mergeInto, prepared));
    } catch (error) {
      if (!owner()) pushToast(`Rename failed: ${String(error)}`, "error");
      throw error;
    }
    if (result.kind === "stale") return "uncertain";
    if (result.value.outcome === "unchanged") return "unchanged";
    const reloads = forgetMovedPages(result.value.touched);
    refreshRenamedNavigation?.(from, to, target);
    await reloadRewrittenPages(reloads);
    return result.value.outcome;
  } finally {
    release();
  }
}

/** Save every pending edit. The rename reads referring pages from disk, so an
 * edit that cannot be saved matters only on a page the rename would change:
 * the renamed page, a namespace child, or one whose unsaved text mentions the
 * old name (a reference that exists only in memory would be missed). Those
 * refuse; every other stuck page's file is returned for the backend to leave
 * alone. O(stuck pages' blocks) after the flush. */
async function unsavedPathsFor(from: string): Promise<string[] | { unsaved: string; mentions: boolean }> {
  if (await flushAll()) return [];
  const renamed = pageIdentityKey(from);
  const mention = from.trim().toLowerCase().normalize("NFC");
  const paths: string[] = [];
  for (const name of new Set([...dirtyPages(), ...savingPages(), ...conflicts()])) {
    const key = pageIdentityKey(name);
    if (key === renamed || key.startsWith(`${renamed}/`)) return { unsaved: name, mentions: false };
    if (mention && pageText(name).toLowerCase().normalize("NFC").includes(mention)) return { unsaved: name, mentions: true };
    const path = pageByName(name)?.id;
    if (path) paths.push(path);
  }
  return paths;
}

/** Everything a loaded page holds in memory: its header and every block. */
function pageText(name: string): string {
  const page = pageByName(name);
  if (!page) return "";
  const parts = [page.preBlock ?? ""];
  const visit = (id: string) => {
    const node = doc.byId[id];
    if (!node) return;
    parts.push(node.raw);
    node.children.forEach(visit);
  };
  page.roots.forEach(visit);
  return parts.join("\n");
}

/** The backend rewrote files through its self-write guard, which suppresses the
 * watcher, so each loaded touched page is stale. A moved (or merge-trashed)
 * page leaves the working set under its old name; the caller opens the new
 * one. Rewritten pages are returned for reload. Their undo is dropped either
 * way: replaying it would restore the pre-rename text. */
function forgetMovedPages(touched: readonly RenameTouchedPage[]): { name: string; path: string }[] {
  const reloads: { name: string; path: string }[] = [];
  for (const page of touched) {
    const loaded = doc.pages.find((candidate) => candidate.id === page.path);
    if (!loaded) continue;
    if (page.moved) forgetPage(loaded.name);
    else {
      invalidateUndoForPage(loaded.name);
      reloads.push({ name: loaded.name, path: page.path });
    }
  }
  return reloads;
}

/** Reload each rewritten page from disk while it is still clean; one that
 * cannot be re-read is dropped from the working set instead. Edits are frozen,
 * so none was edited during the rename; a page edited anyway keeps its edit
 * and its guarded save meets the rewrite as an ordinary conflict. */
async function reloadRewrittenPages(pages: readonly { name: string; path: string }[]): Promise<void> {
  const owner = graphOwner();
  await Promise.all(pages.map(async (page) => {
    const current = () => owner() && pageByName(page.name)?.id === page.path;
    try {
      const result = await readOwned(owner, backend().getPageByPath(page.path));
      if (result.kind === "stale" || !current()) return;
      if (result.value) {
        reloadPageIfStillSafe(page.name, toLoadablePage(result.value, page.name));
        return;
      }
    } catch {
      // Fall through: a page that cannot be re-read must not stay stale.
    }
    // Unreadable or gone: drop the clean stale copy so the next view reads disk.
    if (current() && reloadDisposition(page.name) === "reload") forgetPage(page.name);
  }));
}
