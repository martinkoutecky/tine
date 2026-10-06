import { batch } from "solid-js";
import { backend } from "../backend";
import { bindingOwner, graphOwner, readOwned, writeOwned, type Owner } from "../owned";
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
import { pageRefsInText } from "../render/pageRefs";
import type { Format } from "../render/ast";
import type { RenameDone, RenameTouchedPage } from "../types";
import { graphMeta, setGraphMeta } from "../graphSession";
import { activeElement, isHTMLElementNode } from "../windowRealm";

let refreshRenamedNavigation: ((from: string, to: string, target?: PageTarget) => void) | null = null;

/** The app supplies route/index effects; the document intent owns the flush,
 * edit freeze, disk rewrite and the refresh of the pages it touched. */
export function installRenameRefreshHandler(handler: (from: string, to: string, target?: PageTarget) => void): void {
  refreshRenamedNavigation = handler;
}

/** What `renamePageOnDisk` did. The backend's outcome when it ran; otherwise
 *  `busy` (another graph rewrite is in progress), `{ unsaved }` (the name of a
 *  loaded page whose edits could not be saved and the rename would change it:
 *  it is the renamed page or a namespace child, or `mentions` is true and its
 *  unsaved text references one of them) or `stale` (graph ownership retired
 *  first), all with nothing written; or
 *  `uncertain`: ownership retired after the backend call was made, so the
 *  rename may have committed and disk must be checked. */
export type DiskRename = RenameDone["outcome"] | "busy" | { unsaved: string; mentions: boolean } | "stale" | "uncertain";

/** Freeze user edits, save every pending edit, then ask the backend to rename
 * a page and rewrite references across the graph (merging into `mergeInto`,
 * the confirmed owner of `to`, when given). A page whose edits cannot be saved
 * blocks the rename only when the rename would change it (GH #535); any other
 * such page keeps its unsaved edits, and the backend refuses to rewrite its
 * file (that refusal rejects, naming the page: a stuck merge target, or a
 * stuck page whose file, not its unsaved text, references the old name).
 * Backend errors reject. After a rename or merge, drop the pages it moved under
 * their old names, reload the clean pages it rewrote (their undo is dropped),
 * and refresh navigation; every other loaded page keeps its state, unsaved
 * edits and undo included. `unchanged` touches nothing.
 * Cost grows with graph pages and references, plus one page read per loaded
 * rewritten page. A backend failure can require inspecting disk before
 * retrying. The rename's own write and its bookkeeping (forgetting moved
 * pages, reloading rewritten ones) are owned by the graph binding (R4), so a
 * repaint during the rename cannot skip them. The refresh retires every
 * display owner captured before it, so a caller that must act on success (open
 * the page, confirm) receives the display owner captured after the refresh
 * through `onRefreshed`. Navigation in that
 * callback shares the refresh batch, so views read only the final route. */
export async function renamePageOnDisk(
  from: string, to: string, target?: PageTarget, mergeInto?: string, onRefreshed?: (owner: Owner) => void,
): Promise<DiskRename> {
  if (graphRewriteFrozen()) return "busy";
  // Blur is synchronous: commit the current editor buffer before closing the
  // write gate, with no await or input event between the two steps.
  const focused = typeof document !== "undefined" ? activeElement() : null;
  if (isHTMLElementNode(focused)) focused.blur();
  const release = tryFreezeGraphRewrite();
  if (!release) return "busy";
  const owner = bindingOwner();
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
    // og 21a (master a8fd4230d): files mid-merge keep their old references.
    const skipped = result.value.skipped_conflicted_referrers ?? [];
    if (skipped.length) pushToast(`${skipped.length === 1 ? "One page still mid-merge keeps" : `${skipped.length} pages still mid-merge keep`} ${skipped.length === 1 ? "its" : "their"} references to “${from}”: ${skipped.join(", ")}. Resolve the merge, then update them.`, "warn", { sticky: true });
    // The rename moved `:default-home` in its own transaction (OG
    // `rename-page-aux`); an own write raises no config event, so take it in.
    const home = result.value.home_page;
    const meta = graphMeta();
    if (home && meta) setGraphMeta({ ...meta, default_home: home });
    const reloads = batch(() => {
      const pages = forgetMovedPages(result.value.touched);
      refreshRenamedNavigation?.(from, to, target);
      onRefreshed?.(graphOwner());
      return pages;
    });
    await reloadRewrittenPages(reloads);
    return result.value.outcome;
  } finally {
    release();
  }
}

/** Save every pending edit. The rename reads referring pages from disk, so an
 * edit that cannot be saved matters only on a page the rename would change:
 * the renamed page, a namespace child, or one whose unsaved text references
 * the renamed page or a namespace child (a reference that exists only in
 * memory would be missed). References are the ones the backend rewrites, read
 * off the lsdoc parse and compared by page key; prose that merely contains the
 * name does not count. Those refuse; every other stuck page's file is returned
 * for the backend to leave alone. O(stuck pages' blocks) parses after the flush. */
async function unsavedPathsFor(from: string): Promise<string[] | { unsaved: string; mentions: boolean }> {
  if (await flushAll()) return [];
  const renamed = pageIdentityKey(from);
  const rewritten = (name: string) => {
    const key = pageIdentityKey(name);
    return key === renamed || key.startsWith(`${renamed}/`);
  };
  const paths: string[] = [];
  for (const name of new Set([...dirtyPages(), ...savingPages(), ...conflicts()])) {
    if (rewritten(name)) return { unsaved: name, mentions: false };
    if (pageTexts(name).some((text) => pageRefsInText(text.raw, text.format).some(rewritten)))
      return { unsaved: name, mentions: true };
    const path = pageByName(name)?.id;
    if (path) paths.push(path);
  }
  return paths;
}

/** Everything a loaded page holds in memory: its header and every block. */
function pageTexts(name: string): { raw: string; format: Format }[] {
  const page = pageByName(name);
  if (!page) return [];
  const texts = page.preBlock ? [{ raw: page.preBlock, format: page.format }] : [];
  const visit = (id: string) => {
    const node = doc.byId[id];
    if (!node) return;
    texts.push({ raw: node.raw, format: page.format });
    node.children.forEach(visit);
  };
  page.roots.forEach(visit);
  return texts;
}

/** The backend rewrote files through its self-write guard, which suppresses the
 * watcher, so each loaded touched page is stale. A moved (or merge-trashed)
 * page leaves the working set under its old name; the caller opens the new
 * one. Rewritten pages are returned for reload. Their undo is dropped either
 * way: replaying it would restore the pre-rename text. */
function forgetMovedPages(touched: readonly RenameTouchedPage[]): { name: string; path: string }[] {
  const reloads: { name: string; path: string }[] = [];
  const loadedByPath = new Map<string, (typeof doc.pages)[number]>();
  for (const page of doc.pages) {
    if (page.id !== undefined && !loadedByPath.has(page.id)) loadedByPath.set(page.id, page);
  }
  for (const page of touched) {
    const loaded = loadedByPath.get(page.path);
    if (!loaded) continue;
    if (page.moved) {
      loadedByPath.delete(page.path);
      forgetPage(loaded.name);
    }
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
  const owner = bindingOwner();
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
