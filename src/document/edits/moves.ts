import { doc, setDoc, formatForBlock, formatForPage, pageByName, DocState } from "../model";
import { blockWritable, pageWritable, orderListTypeFromRaw, rawWithInheritedOrderListType } from "./properties";
import { produce } from "solid-js/store";
import { markDirty, persistTogether, refuseConflictedMove } from "../save/engine";
import { captureBinding, stillBound } from "../../binding";
import { pushMoveSelectionUndo, pushUndo } from "../history";
import { createSignal } from "solid-js";
import { rootsOf, nextVisible, existingSubtreeFits } from "../tree";
import { topSelected } from "./selection";
import { pushToast } from "../../toasts";

/** Move a block under `newParent` (or, when `newParent` is null, to the roots of
 *  `targetPage` — pass the drop target's page so a root-to-root drop across pages
 *  lands on the RIGHT page instead of defaulting back to the source). A depth
 *  refusal returns false and shows a toast. */
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
        reassignPage(s, id, newPage);
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

interface RelativeMovePlan {
  roots: string[];
  sourcePages: string[];
  destinationPage: string;
}

/** Build the complete target-relative move plan without mutating (master
 * 6eea5b70c, GH #240). Captured IDs are stable-deduped, then descendants of
 * another captured ID are subsumed. Any malformed tree, read-only page, a
 * target inside a moved subtree, or an outline-depth overflow refuses the whole
 * move before anything changes. Only the depth refusal is the user's to learn
 * about ("too-deep", checked last so a no-op drop into its own subtree stays
 * silent); the rest are no-op gestures or stale ids, silent as in `moveBlock`. */
function relativeMovePlan(
  capturedIds: readonly string[],
  targetId: string,
  position: "before" | "after" | "child",
): RelativeMovePlan | "too-deep" | null {
  const unique = [...new Set(capturedIds)];
  if (!unique.length || unique.some((id) => !doc.byId[id])) return null;
  const captured = new Set(unique);
  const roots: string[] = [];
  for (const id of unique) {
    const seen = new Set([id]);
    let parent = doc.byId[id].parent;
    let subsumed = false;
    while (parent !== null) {
      if (seen.has(parent)) return null;
      seen.add(parent);
      if (captured.has(parent)) { subsumed = true; break; }
      const ancestor = doc.byId[parent];
      if (!ancestor) return null;
      parent = ancestor.parent;
    }
    if (!subsumed) roots.push(id);
  }
  if (!roots.length) return null;

  const target = doc.byId[targetId];
  if (!target || !pageWritable(target.page)) return null;
  const destinationParent = target.parent;
  if (destinationParent !== null) {
    const parent = doc.byId[destinationParent];
    if (!parent || parent.page !== target.page || !blockWritable(destinationParent)) return null;
  }
  const siblingsOf = (parent: string | null, page: string) => parent === null ? pageByName(page)?.roots : doc.byId[parent]?.children;
  const targetSiblings = siblingsOf(destinationParent, target.page);
  if (!targetSiblings || targetSiblings.filter((id) => id === targetId).length !== 1) return null;

  const moved = new Set<string>();
  const visit = (id: string, page: string, ancestry: Set<string>): boolean => {
    const node = doc.byId[id];
    if (!node || node.page !== page || moved.has(id) || ancestry.has(id)) return false;
    moved.add(id);
    if (new Set(node.children).size !== node.children.length) return false;
    const nextAncestry = new Set(ancestry).add(id);
    return node.children.every((childId) => doc.byId[childId]?.parent === id && visit(childId, page, nextAncestry));
  };
  const sourcePages: string[] = [];
  let tooDeep = false;
  for (const id of roots) {
    const node = doc.byId[id];
    if (!blockWritable(id)) return null;
    const siblings = siblingsOf(node.parent, node.page);
    if (!siblings || siblings.filter((sibling) => sibling === id).length !== 1) return null;
    if (node.parent !== null && doc.byId[node.parent]?.page !== node.page) return null;
    if (!visit(id, node.page, new Set())) return null;
    // A nested drop lands UNDER the target, one level deeper than its siblings.
    if (!existingSubtreeFits(id, position === "child" ? targetId : destinationParent)) tooDeep = true;
    sourcePages.push(node.page);
  }
  if (moved.has(targetId)) return null;
  const uniqueSources = [...new Set(sourcePages)];
  if (uniqueSources.some((page) => !pageWritable(page))) return null;
  if (tooDeep) return "too-deep";
  return { roots, sourcePages: uniqueSources, destinationPage: target.page };
}

/** Move captured selection roots together before/after a live target ID, as
 * one transaction (one undo unit, one publication). Cross-page moves persist
 * every touched page through the same `persistTogether` group as `moveBlock`,
 * so the destination and emptied sources are saved as one unit (an honest
 * concurrent instance or external editor sees either the whole move or none);
 * a page with an unresolved conflict refuses the move (`refuseConflictedMove`,
 * same scenario as a single-block drag). */
export async function moveBlocksRelative(
  capturedIds: readonly string[],
  targetId: string,
  position: "before" | "after" | "child",
): Promise<boolean> {
  const plan = relativeMovePlan(capturedIds, targetId, position);
  if (plan === "too-deep") {
    // Same notice as `moveBlock` (C3Y Y5: a refused drop used to vanish).
    pushToast("Outline is too deep to move", "error");
    return false;
  }
  if (!plan) return false;
  const pages = [...new Set([plan.destinationPage, ...plan.sourcePages])];
  const crossSources = plan.sourcePages.filter((page) => page !== plan.destinationPage);
  if (crossSources.length && refuseConflictedMove(pages)) return false;
  const destinationFormat = formatForPage(plan.destinationPage);
  const movedRaw = new Map(plan.roots.map((id) => {
    const sourceRaw = doc.byId[id].raw;
    const raw = orderListTypeFromRaw(sourceRaw, formatForBlock(id)) !== null
      ? sourceRaw
      : rawWithInheritedOrderListType(sourceRaw, destinationFormat, targetId);
    return [id, raw] as const;
  }));
  pushUndo("move-selection-relative", pages);
  setDoc(produce((state) => {
    const siblingsFor = (id: string): string[] => {
      const node = state.byId[id];
      return node.parent === null
        ? state.pages.find((page) => page.name === node.page)!.roots
        : state.byId[node.parent].children;
    };
    for (const id of plan.roots) {
      const siblings = siblingsFor(id);
      siblings.splice(siblings.indexOf(id), 1);
    }
    const target = state.byId[targetId];
    // "child": append under the target (OG's nested drop, GH #326).
    const destinationParent = position === "child" ? targetId : target.parent;
    const destination = destinationParent === null
      ? state.pages.find((page) => page.name === target.page)!.roots
      : state.byId[destinationParent].children;
    const targetIndex = position === "child" ? destination.length : destination.indexOf(targetId);
    for (const id of plan.roots) {
      state.byId[id].parent = destinationParent;
      state.byId[id].raw = movedRaw.get(id)!;
    }
    destination.splice(targetIndex + (position === "after" ? 1 : 0), 0, ...plan.roots);
    for (const id of plan.roots) reassignPage(state, id, plan.destinationPage);
  }));
  if (crossSources.length) {
    void persistTogether(pages, ["move-blocks", "save-block"], crossSources.map((source) => [source, plan.destinationPage] as [string, string]));
  } else {
    markDirty(plan.destinationPage, ["move-blocks", "save-block"]);
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

interface OutlineStepPlan {
  parent: string | null;
  index: number;
}

/** OG's move-up/down is an outline-order operation, not merely a sibling swap.
 * At a child-list edge it crosses into the adjacent parent sibling: moving down
 * enters the next sibling as its first child; the inverse up move enters the
 * previous sibling as its last child. Root edges remain available to the
 * journal-feed cross-page route below (GH #312). */
function outlineStepPlan(id: string, dir: 1 | -1): OutlineStepPlan | null {
  const node = doc.byId[id];
  if (!node) return null;
  const sibs = rootsOf(id);
  const i = sibs.indexOf(id);
  if (i < 0) return null;
  const siblingIndex = i + dir;
  if (siblingIndex >= 0 && siblingIndex < sibs.length) {
    return { parent: node.parent, index: siblingIndex };
  }
  if (node.parent === null) return null;
  const parent = doc.byId[node.parent];
  if (!parent) return null;
  const parentSiblings = rootsOf(node.parent);
  const parentIndex = parentSiblings.indexOf(node.parent);
  const adjacentParent = parentSiblings[parentIndex + dir];
  if (!adjacentParent || !doc.byId[adjacentParent]) return null;
  return {
    parent: adjacentParent,
    index: dir === -1 ? doc.byId[adjacentParent].children.length : 0,
  };
}

/** Move a block one OG outline step (sibling swap, or across a child-list edge
 *  into the adjacent parent sibling). One undo entry; returns false (no change)
 *  when read-only or no step exists in `dir`. A reparented block inherits the
 *  new parent's list numbering. */
export function moveItem(id: string, dir: 1 | -1): boolean {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return false;
  const plan = outlineStepPlan(id, dir);
  if (!plan) return false;
  const movedRaw = plan.parent !== node.parent
    ? rawWithInheritedOrderListType(node.raw, formatForPage(node.page), plan.parent)
    : node.raw;
  pushUndo("move-item", [node.page]);
  setDoc(
    produce((s) => {
      const source =
        node.parent === null
          ? s.pages[s.pages.findIndex((p) => p.name === node.page)].roots
          : s.byId[node.parent!].children;
      const from = source.indexOf(id);
      if (from < 0) return;
      source.splice(from, 1);
      const destination = plan.parent === null
        ? s.pages[s.pages.findIndex((p) => p.name === node.page)].roots
        : s.byId[plan.parent].children;
      s.byId[id].parent = plan.parent;
      s.byId[id].raw = movedRaw;
      destination.splice(Math.max(0, Math.min(plan.index, destination.length)), 0, id);
    })
  );
  markDirty(node.page, plan.parent !== node.parent ? ["move-blocks", "save-block"] : "move-blocks");
  return true;
}

/** Can a block move one OG outline step in `dir`? */
function canMoveItem(id: string, dir: 1 | -1): boolean {
  return outlineStepPlan(id, dir) !== null;
}

/** Selection movement is still batched as sibling-array swaps. Structural edge
 * crossing is handled separately from its root/day boundary below. */
function canMoveSelectionWithinSiblings(id: string, dir: 1 | -1): boolean {
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

/** The one door for moving root blocks across a journal-feed day boundary
 *  (`moveBlockFeed` and `moveSelectionItems` both end here). It resolves the
 *  neighbour day, which may await the feed extender, and then RE-CHECKS every
 *  fact its plan rested on before writing: the graph binding (twice, around the
 *  conflict check), both pages writable, neither page conflicted, and that `ids`
 *  are still roots of `from`. The write itself is one `persistTogether` group.
 *  A caller may not repeat this list: a caller that did once drifted from its
 *  twin (master eba7c56b2 H1-H5; og answers them through SaveGroups, not a
 *  ported coordinator). Resolves to whether the blocks crossed. */
async function crossDayMove(
  ids: string[],
  from: string,
  dir: 1 | -1,
  undoKind: "move-cross" | "move-sel-cross",
  binding: ReturnType<typeof captureBinding>,
): Promise<boolean> {
  const target = await feedNeighbor(from, dir);
  if (!stillBound(binding)) return false;
  if (!target || target === from || !pageWritable(target)) return false;
  if (refuseConflictedMove([from, target])) return false;
  if (!stillBound(binding)) return false;
  if (!stillRootsOf(ids, from)) return false; // moved or vanished during the await (H3)
  if (!pageWritable(from) || !pageWritable(target)) return false;
  pushUndo(undoKind, [from, target]);
  crossMoveBlocks(ids, from, target, dir);
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
  if (!node || !blockWritable(id)) return "none";
  if (canMoveItem(id, dir)) {
    moveItem(id, dir);
    return "within";
  }
  if (node.parent !== null) return "none"; // nested block at a child-list edge: stop
  return (await crossDayMove([id], node.page, dir, "move-cross", binding)) ? "crossed" : "none";
}

/** Move every top-level selected block up/down by one slot, preserving the
 *  selection; at a day boundary the whole group crosses into the adjacent day. */
export async function moveSelectionItems(dir: 1 | -1) {
  const binding = captureBinding();
  const ids = topSelected(); // document order: ids[0] topmost, last bottommost
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
  const lead = dir === 1 ? ids[ids.length - 1] : ids[0];
  if (canMoveSelectionWithinSiblings(lead, dir)) {
    // Batch the whole selection into ONE undo entry + ONE produce. Doing it
    // per-block (a moveItem call each) snapshots the entire working set K times —
    // a 15-block nudge became 15 full clones, the visible jank. Going down, move
    // the bottom-most first so they don't collide; up, the top.
    const ordered = dir === 1 ? [...ids].reverse() : ids;
    const pages = [...new Set(ordered.map((id) => doc.byId[id]?.page).filter(Boolean) as string[])];
    if (pages.length > 1 && refuseConflictedMove(pages)) return;
    pushMoveSelectionUndo(ids, pages); // one step per held-key burst, scoped to the touched pages
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
  await crossDayMove(ids, page, dir, "move-sel-cross", binding);
}
