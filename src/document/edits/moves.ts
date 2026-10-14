import { msg } from "../../i18n";
import { doc, pageByName, DocState } from "../model";
import { refuseConflictedMove } from "../host/wiring";
import { captureBinding, bindingCurrent } from "../../binding";
import { pushMoveSelectionUndo, pushUndo } from "../history";
import { createSignal } from "solid-js";
import { nextVisible } from "../tree";
import { topSelected } from "./selection";
import { pushToast } from "../../toasts";
import { applyOutlineChange, markOutlineDirty, outlineChange, outlineChangePages, type OutlineChange, type OutlineRefusal, type OutlinePlace } from "../kernel";

/** Run a computed move: a refusal stops before the undo entry (only the depth
 *  cap is the user's to learn about, as a toast); a move between pages first
 *  checks that neither page holds an unresolved conflict (STEP3 §8). Resolves
 *  true when applied, false on the depth cap or a conflict, undefined on a
 *  silent refusal (a no-op gesture or stale ids). */
function commitMove(change: OutlineChange | { refused: OutlineRefusal }, undoKind: string): boolean | undefined {
  if ("refused" in change) {
    if (change.refused !== "depth_cap") return undefined;
    pushToast(msg("moves-outline-is-too-deep-to-move"), "error");
    return false;
  }
  const pages = outlineChangePages(change);
  if (change.transfers.length && refuseConflictedMove(pages)) return false;
  pushUndo(undoKind, pages);
  applyOutlineChange(change);
  markOutlineDirty(change);
  return true;
}

/** Move a block under `newParent` (or, when `newParent` is null, to the roots of
 *  `targetPage` — pass the drop target's page so a root-to-root drop across pages
 *  lands on the RIGHT page instead of defaulting back to the source) at `index`
 *  of that list as it stands before the move. A depth refusal returns false and
 *  shows a toast. */
export async function moveBlock(
  id: string,
  newParent: string | null,
  index: number,
  targetPage?: string,
  dropTargetId?: string,
) {
  const node = doc.byId[id];
  if (!node) return;
  // A root drop has no parent to read the page from — use the explicit target
  // page (the day/page the drop landed on); fall back to the source page only if
  // the caller didn't supply one (a same-page reorder).
  const newPage = newParent ? doc.byId[newParent]?.page : (targetPage ?? node.page);
  if (!newPage) return;
  const to: OutlinePlace = { at: "slot", parent: newParent, page: newPage, index };
  const change = outlineChange({ verb: "move_to", keys: [id], to, inherit_from: dropTargetId ?? newParent }, [node.page, newPage]);
  return commitMove(change, "move");
}

/** Move captured selection roots together before/after a live target ID, or
 * after its last child, as one transaction (one undo unit, one publication;
 * master 6eea5b70c, GH #240). The kernel's `move_to` dedupes the captured IDs,
 * lets a captured descendant go with its ancestor, and refuses the whole move
 * before anything changes. Cross-page moves persist every touched page through
 * one transfer (`markOutlineDirty` → `persistTransfer`): one host move per
 * source, so a crash between steps can leave a block in both pages, never in
 * neither (STEP3 §8). */
export async function moveBlocksRelative(
  capturedIds: readonly string[],
  targetId: string,
  position: "before" | "after" | "child",
): Promise<boolean> {
  const target = doc.byId[targetId];
  if (!target) return false;
  const pages = [target.page, ...capturedIds.map((id) => doc.byId[id]?.page ?? target.page)];
  // "child": append under the target (OG's nested drop, GH #326).
  const to: OutlinePlace = { at: position === "child" ? "last_child" : position, key: targetId };
  const change = outlineChange({ verb: "move_to", keys: capturedIds, to, inherit_from: targetId }, pages);
  return (await commitMove(change, "move-selection-relative")) === true;
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

/** Move a block one OG outline step (sibling swap, or across a child-list edge
 *  into the adjacent parent sibling): the kernel's `move_up_down`. One undo
 *  entry; returns false (no change) when read-only or no step exists in `dir`.
 *  A reparented block inherits the new parent's list numbering. */
export function moveItem(id: string, dir: 1 | -1): boolean {
  return applyStep(id, dir) === true;
}

/** The step change for `id`, applied; the refusal otherwise. */
function applyStep(id: string, dir: 1 | -1): true | OutlineRefusal {
  const node = doc.byId[id];
  if (!node) return "incomplete";
  const change = outlineChange({ verb: "move_up_down", keys: [id], up: dir === -1 }, [node.page]);
  if ("refused" in change) return change.refused;
  pushUndo("move-item", [node.page]);
  applyOutlineChange(change);
  markOutlineDirty(change);
  return true;
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

/** True while every id is still a root of `fromPage`. A cross-day move plans
 *  before awaiting the feed extender and must re-check its plan after it: a
 *  second nudge issued meanwhile may already have moved these blocks, and
 *  applying the stale plan would put them into the target day twice (duplicate
 *  content and `id::`; master eba7c56b2 H3). O(ids + fromPage roots). */
function stillRootsOf(ids: readonly string[], fromPage: string): boolean {
  const from = pageByName(fromPage);
  if (!from) return false;
  const roots = new Set(from.roots);
  return ids.every((id) => roots.has(id) && doc.byId[id]?.page === fromPage && doc.byId[id]?.parent === null);
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

/** The one door for moving root blocks across a journal-feed day boundary
 *  (`moveBlockFeed` and `moveSelectionItems` both end here). It resolves the
 *  neighbour day, which may await the feed extender, and then RE-CHECKS every
 *  fact its plan rested on before writing: the graph binding (twice, around the
 *  conflict check), that `ids` are still roots of `from`, both pages writable
 *  (the kernel's refusal), and neither page conflicted. The write itself is one
 *  transfer (`markOutlineDirty` → `persistTransfer`).
 *  A caller may not repeat this list: a caller that did once drifted from its
 *  twin (master eba7c56b2 H1-H5; og answers them through one host transfer, not a
 *  ported coordinator). Resolves to whether the blocks crossed. */
async function crossDayMove(
  ids: string[],
  from: string,
  dir: 1 | -1,
  undoKind: "move-cross" | "move-sel-cross",
  binding: ReturnType<typeof captureBinding>,
): Promise<boolean> {
  const target = await feedNeighbor(from, dir);
  if (!bindingCurrent(binding)) return false;
  if (!target || target === from) return false;
  if (!stillRootsOf(ids, from)) return false; // moved or vanished during the await (H3)
  // up → bottom of the day above; down → top of the day below (keep order).
  // The kernel refuses a read-only or frozen page.
  const to: OutlinePlace = { at: "slot", parent: null, page: target, index: dir === -1 ? (pageByName(target)?.roots.length ?? 0) : 0 };
  const change = outlineChange({ verb: "move_to", keys: ids, to }, [from, target]);
  if ("refused" in change) return false;
  if (refuseConflictedMove([from, target])) return false;
  if (!bindingCurrent(binding)) return false;
  pushUndo(undoKind, [from, target]);
  applyOutlineChange(change);
  markOutlineDirty(change);
  return true;
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
  if (!node) return "none";
  const step = applyStep(id, dir);
  if (step === true) return "within";
  // Only a root block at its day's edge crosses; a nested block stops.
  if (step !== "no_outline_step" || node.parent !== null) return "none";
  return (await crossDayMove([id], node.page, dir, "move-cross", binding)) ? "crossed" : "none";
}

/** Move every top-level selected block up/down by one slot, preserving the
 *  selection; at a day boundary the whole group crosses into the adjacent day. */
export async function moveSelectionItems(dir: 1 | -1) {
  const binding = captureBinding();
  const ids = topSelected(); // document order: ids[0] topmost, last bottommost
  if (!ids.length) return;
  const pages = [...new Set(ids.map((id) => doc.byId[id]?.page).filter(Boolean) as string[])];
  // One kernel change for the whole selection: ONE undo entry, ONE store
  // mutation. Per-block moves snapshot the entire working set K times — a
  // 15-block nudge became 15 full clones, the visible jank.
  const change = outlineChange({ verb: "move_up_down", keys: ids, up: dir === -1, within_siblings: true }, pages);
  if (!("refused" in change)) {
    if (pages.length > 1 && refuseConflictedMove(pages)) return;
    pushMoveSelectionUndo(ids, pages); // one step per held-key burst, scoped to the touched pages
    applyOutlineChange(change);
    // Reordering within each page moves no block between pages: one edit per page.
    markOutlineDirty(change);
    return;
  }
  if (change.refused !== "no_outline_step") return;
  // Boundary: cross the whole group into the adjacent day (only if every
  // selected block is a root block on the same feed day).
  const page = pages[0];
  if (pages.length > 1 || ids.some((id) => doc.byId[id].parent !== null)) return;
  await crossDayMove(ids, page, dir, "move-sel-cross", binding);
}
