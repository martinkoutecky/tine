import { doc, setDoc, formatForBlock, formatForPage, DocState } from "../model";
import { blockWritable, pageWritable, orderListTypeFromRaw, rawWithInheritedOrderListType } from "./properties";
import { produce } from "solid-js/store";
import { markDirty, persistTogether, refuseConflictedMove } from "../save/engine";
import { captureBinding, stillBound } from "../../binding";
import { pushUndo } from "../history";
import { createSignal } from "solid-js";
import { rootsOf, nextVisible, existingSubtreeFits } from "../tree";
import { topSelected } from "./selection";
import { pushToast } from "../../toasts";

/** Move a block to be a child of `newParent` (or root of its page) at `index`.
 *  Used by drag-and-drop. */
/** Move without pushing an undo entry (for batched selection ops). */
export function moveBlockInternal(id: string, newParent: string | null, index: number) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id) || (newParent !== null && !blockWritable(newParent))) return;
  let p = newParent;
  while (p !== null) {
    if (p === id) return;
    p = doc.byId[p].parent;
  }
  if (!existingSubtreeFits(id, newParent)) {
    pushToast("Outline is too deep to move", "error");
    return false;
  }
  const oldPage = node.page;
  const newPage = newParent ? doc.byId[newParent].page : oldPage;
  if (newPage !== oldPage && refuseConflictedMove([oldPage, newPage])) return;
  setDoc(
    produce((s) => {
      const oldArr =
        node.parent === null
          ? s.pages[s.pages.findIndex((x) => x.name === oldPage)].roots
          : s.byId[node.parent!].children;
      const from = oldArr.indexOf(id);
      oldArr.splice(from, 1);
      s.byId[id].parent = newParent;
      const newArr =
        newParent === null
          ? s.pages[s.pages.findIndex((x) => x.name === newPage)].roots
          : s.byId[newParent].children;
      let idx = index;
      if (oldArr === newArr && from < idx) idx -= 1;
      newArr.splice(Math.max(0, Math.min(idx, newArr.length)), 0, id);
      if (newPage !== oldPage) {
        const reassign = (bid: string) => {
          s.byId[bid].page = newPage;
          s.byId[bid].children.forEach(reassign);
        };
        reassign(id);
      }
    })
  );
  if (newPage !== oldPage) void persistTogether([oldPage, newPage], "move-blocks", [[oldPage, newPage]]);
  else markDirty(oldPage, "move-blocks");
  return true;
}

/** Move a block under `newParent` (or, when `newParent` is null, to the roots of
 *  `targetPage` — pass the drop target's page so a root-to-root drop across pages
 *  lands on the RIGHT page instead of defaulting back to the source). */
export async function moveBlock(
  id: string,
  newParent: string | null,
  index: number,
  targetPage?: string,
  dropTargetId?: string,
) {
  const binding = captureBinding();
  const node = doc.byId[id];
  if (!node) return;
  // Don't drop a block into its own descendant.
  let p = newParent;
  while (p !== null) {
    if (p === id) return;
    p = doc.byId[p].parent;
  }
  if (!existingSubtreeFits(id, newParent)) {
    pushToast("Outline is too deep to move", "error");
    return false;
  }
  const oldPage = node.page;
  // A root drop has no parent to read the page from — use the explicit target
  // page (the day/page the drop landed on); fall back to the source page only if
  // the caller didn't supply one (a same-page reorder).
  const newPage = newParent ? doc.byId[newParent].page : (targetPage ?? oldPage);
  if (!pageWritable(oldPage) || !pageWritable(newPage)) return;
  if (newPage !== oldPage && refuseConflictedMove([oldPage, newPage])) return;
  if (!stillBound(binding)) return;
  if (!doc.byId[id]) return; // block vanished during the async flush
  if (!pageWritable(oldPage) || !pageWritable(newPage)) return;
  const sourceFormat = formatForBlock(id);
  const destinationFormat = formatForPage(newPage);
  const inheritanceTarget = dropTargetId ?? newParent;
  // A cross-format move already preserves the source raw verbatim; only a newly
  // inherited property is emitted in the destination page's syntax.
  const movedRaw = orderListTypeFromRaw(doc.byId[id].raw, sourceFormat) !== null
    ? doc.byId[id].raw
    : rawWithInheritedOrderListType(doc.byId[id].raw, destinationFormat, inheritanceTarget);
  // Drag-move can cross pages → snapshot both source and destination.
  pushUndo("move", [...new Set([oldPage, newPage])]);
  setDoc(
    produce((s) => {
      const oldArr =
        node.parent === null
          ? s.pages[s.pages.findIndex((x) => x.name === oldPage)].roots
          : s.byId[node.parent!].children;
      const from = oldArr.indexOf(id);
      oldArr.splice(from, 1);
      s.byId[id].parent = newParent;
      s.byId[id].raw = movedRaw;
      const newArr =
        newParent === null
          ? s.pages[s.pages.findIndex((x) => x.name === newPage)].roots
          : s.byId[newParent].children;
      let idx = index;
      if (oldArr === newArr && from < idx) idx -= 1;
      newArr.splice(Math.max(0, Math.min(idx, newArr.length)), 0, id);
      // Reassign the moved subtree to the target page.
      if (newPage !== oldPage) {
        const reassign = (bid: string) => {
          s.byId[bid].page = newPage;
          s.byId[bid].children.forEach(reassign);
        };
        reassign(id);
      }
    })
  );
  if (newPage !== oldPage) {
    void persistTogether([oldPage, newPage], ["move-blocks", "save-block"], [[oldPage, newPage]]);
  } else {
    markDirty(oldPage, ["move-blocks", "save-block"]);
  }
  return true;
}

/** Move a block up/down among its siblings (mod+Up/Down). Keyed <For> keeps the
 *  DOM node — so if the block is being edited, the textarea + caret survive. */
// During a block-move reorder the textarea momentarily blurs; this flag tells
// the editor's onBlur to keep edit mode (the move handler refocuses + restores
// the caret right after).
// A reorder only keeps the editor transiently blurred for one animation frame.
// Keep its page ownership: watcher/feed refreshes for another page must not be
// held hostage by a sidebar or split-pane reorder.
let blockMovingPage: string | null = null;
// Feed refresh ownership observes the end of a page-scoped drag.  Keep the
// inexpensive page check above, but make its lifecycle observable so a deferred
// restart is released by the move itself rather than a coincidental later event.
const [blockMoveRev, setBlockMoveRev] = createSignal(0);
export function isBlockMoving(page?: string): boolean {
  blockMoveRev();
  return blockMovingPage !== null && (page === undefined || blockMovingPage === page);
}
export function setBlockMoving(v: boolean, page?: string): void {
  blockMovingPage = v ? (page ?? blockMovingPage ?? "") : null;
  setBlockMoveRev((n) => n + 1);
}

/** Keep watcher/feed reloads away from a transiently blurred move, including
 * when the move or its caret-restoration callback rejects. */
export async function withBlockMoving<T>(page: string, move: () => T | Promise<T>): Promise<T> {
  setBlockMoving(true, page);
  try {
    return await move();
  } finally {
    setBlockMoving(false);
  }
}

export function moveItem(id: string, dir: 1 | -1) {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return;
  const sibs = rootsOf(id);
  const i = sibs.indexOf(id);
  const ni = i + dir;
  if (ni < 0 || ni >= sibs.length) return;
  pushUndo("move-item", [node.page]);
  setDoc(
    produce((s) => {
      const arr =
        node.parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === node.page)].roots
          : s.byId[node.parent!].children;
      arr.splice(i, 1);
      arr.splice(ni, 0, id);
    })
  );
  markDirty(node.page, "move-blocks");
}

/** Can a block move one slot in `dir` within its sibling list? */
function canMoveItem(id: string, dir: 1 | -1): boolean {
  const sibs = rootsOf(id);
  const ni = sibs.indexOf(id) + dir;
  return ni >= 0 && ni < sibs.length;
}

// The journal feed treats its days as one continuous list: a root block at the
// top/bottom of a day moves into the adjacent *displayed* day (feed order, not
// calendar — non-displayed days like an uncreated 16th are skipped). Page.tsx
// registers a loader so a down-move past the last loaded day pulls in more.
let feedExtender: (() => Promise<boolean>) | null = null;
export function setFeedExtender(fn: (() => Promise<boolean>) | null): void {
  feedExtender = fn;
}

/** Reassign a block subtree's `page` (used when it crosses to another day). */
export function reassignPage(s: DocState, id: string, page: string) {
  s.byId[id].page = page;
  for (const c of s.byId[id].children) reassignPage(s, c, page);
}

/** Move root blocks `ids` (document order) to the start (down) / end (up) of
 *  `toPage`, removing them from `fromPage`. Both pages must be loaded. */
function crossMoveBlocks(ids: string[], fromPage: string, toPage: string, dir: 1 | -1) {
  setDoc(
    produce((s) => {
      const from = s.pages.find((p) => p.name === fromPage);
      const to = s.pages.find((p) => p.name === toPage);
      if (!from || !to) return;
      const idset = new Set(ids);
      from.roots = from.roots.filter((x) => !idset.has(x));
      // up → bottom of the day above; down → top of the day below (keep order).
      if (dir === -1) to.roots.push(...ids);
      else to.roots.unshift(...ids);
      for (const id of ids) {
        s.byId[id].parent = null;
        reassignPage(s, id, toPage);
      }
    })
  );
  void persistTogether([fromPage, toPage], "move-blocks", [[fromPage, toPage]]);
}

/** Resolve the adjacent feed day for a root block at the page boundary, loading
 *  older days if a down-move runs off the last loaded one. Returns the target
 *  page name, or null if there's nowhere to go. */
async function feedNeighbor(page: string, dir: 1 | -1): Promise<string | null> {
  let fi = doc.feed.indexOf(page);
  if (fi < 0) return null; // not a feed day (e.g. a named page)
  let ti = fi + dir;
  if (ti < 0) return null; // top of the feed (today) — can't go higher
  if (ti >= doc.feed.length) {
    if (dir !== 1 || !feedExtender || !(await feedExtender())) return null;
    fi = doc.feed.indexOf(page);
    ti = fi + dir;
    if (ti < 0 || ti >= doc.feed.length) return null;
  }
  return doc.feed[ti];
}

/** Like `nextVisible`, but when we're at the last LOADED block of the journal feed
 *  it pulls in the next day first (via the feed extender) and returns that day's
 *  first block. This lets Down-arrow keep going past the loaded window — previously
 *  only mouse-wheel scrolling (the LoadMore sentinel) grew the feed, so keyboard nav
 *  dead-ended at the last loaded bullet. Resolves to null when there's genuinely
 *  nothing below (a non-feed page, or the feed is exhausted). */
export async function nextVisibleOrExtend(id: string): Promise<string | null> {
  const direct = nextVisible(id);
  if (direct) return direct;
  const node = doc.byId[id];
  if (!node || doc.feed.indexOf(node.page) < 0) return null; // not a feed day → nothing to load
  if (!feedExtender || !(await feedExtender())) return null; // feed exhausted / no extender
  return nextVisible(id); // the newly-appended day's first block is now loaded
}

/** Pull in the next journal-feed day if there is one; resolves to whether the feed
 *  actually grew. Used by scroll-restore to reach a saved offset that lives in
 *  not-yet-loaded days (the feed otherwise only grows on a mouse-wheel sentinel
 *  hit). No-op (false) on a non-feed page or when the feed is exhausted. */
export async function extendFeedForScroll(): Promise<boolean> {
  return feedExtender ? feedExtender() : false;
}

/** Move a single block one slot, crossing into the adjacent day at a page
 *  boundary. Returns how it moved so the caller can restore the caret. */
export async function moveBlockFeed(id: string, dir: 1 | -1): Promise<"within" | "crossed" | "none"> {
  const binding = captureBinding();
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return "none";
  if (canMoveItem(id, dir)) {
    moveItem(id, dir);
    return "within";
  }
  if (node.parent !== null) return "none"; // nested block at a child-list edge: stop
  const target = await feedNeighbor(node.page, dir);
  if (!stillBound(binding)) return "none";
  if (!target || !pageWritable(target)) return "none";
  if (refuseConflictedMove([node.page, target])) return "none";
  if (!stillBound(binding)) return "none";
  if (!doc.byId[id]) return "none"; // vanished during the flush
  if (!pageWritable(node.page) || !pageWritable(target)) return "none";
  pushUndo("move-cross", [node.page, target]);
  crossMoveBlocks([id], node.page, target, dir);
  return "crossed";
}

/** Move every top-level selected block up/down by one slot, preserving the
 *  selection; at a day boundary the whole group crosses into the adjacent day. */
export async function moveSelectionItems(dir: 1 | -1) {
  const binding = captureBinding();
  const ids = topSelected(); // document order: ids[0] topmost, last bottommost
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
  const lead = dir === 1 ? ids[ids.length - 1] : ids[0];
  if (canMoveItem(lead, dir)) {
    // Batch the whole selection into ONE undo entry + ONE produce. Doing it
    // per-block (a moveItem call each) snapshots the entire working set K times —
    // a 15-block nudge became 15 full clones, the visible jank. Going down, move
    // the bottom-most first so they don't collide; up, the top.
    const ordered = dir === 1 ? [...ids].reverse() : ids;
    const pages = [...new Set(ordered.map((id) => doc.byId[id]?.page).filter(Boolean) as string[])];
    if (pages.length > 1 && refuseConflictedMove(pages)) return;
    pushUndo("move-sel", pages); // scope the undo to the touched pages, not the whole set
    setDoc(
      produce((s) => {
        for (const id of ordered) {
          const node = s.byId[id];
          if (!node) continue;
          const arr =
            node.parent === null
              ? s.pages[s.pages.findIndex((p) => p.name === node.page)].roots
              : s.byId[node.parent].children;
          const i = arr.indexOf(id);
          const ni = i + dir;
          if (i < 0 || ni < 0 || ni >= arr.length) continue;
          arr.splice(i, 1);
          arr.splice(ni, 0, id);
        }
      })
    );
    if (pages.length > 1) void persistTogether(pages, "move-blocks");
    else for (const p of pages) markDirty(p, "move-blocks");
    return;
  }
  // Boundary: cross the whole group into the adjacent day (only if every
  // selected block is a root block on the same feed day).
  const page = doc.byId[ids[0]]?.page;
  if (!page) return;
  if (ids.some((id) => doc.byId[id].parent !== null || doc.byId[id].page !== page)) return;
  const target = await feedNeighbor(page, dir);
  if (!stillBound(binding)) return;
  if (!target || !pageWritable(target)) return;
  if (refuseConflictedMove([page, target])) return;
  if (!stillBound(binding)) return;
  if (!pageWritable(page) || !pageWritable(target)) return;
  pushUndo("move-sel-cross", [page, target]);
  crossMoveBlocks(ids, page, target, dir);
}
