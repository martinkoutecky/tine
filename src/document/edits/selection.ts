import { revealOutlineBlock } from "../../outlineViewport";
import { removeSubtree } from "./subtree";
import { OutlineScope, scopedVisibleOrder, visibleData, visibleOrder, pageVisibleOrder, nextVisible, prevVisible, rootsOf, existingSubtreeFits } from "../tree";
import { doc, setDoc, bumpCollapseEpochs, type DocState } from "../model";
import { createSignal, createRoot, createMemo } from "solid-js";
import { endEdit, editingId } from "../../editorController";
import { installClearOutlineSelection, notifyOutlineSelectionStarted } from "../../modeHooks";
import { clearOnBindingInvalidated } from "../../binding";
import { blockWritable, rawWithHeading, rawWithCollapsed, type HeadingState } from "./properties";
import { formatForBlock } from "../model";
import { pushUndo } from "../history";
import { produce } from "solid-js/store";
import { cycleMarkerSmart } from "../../editor/repeat";
import { workflow } from "../../ui";
import { markDirty, persistTogether } from "../save/engine";
import { copyStripCollapsed, copyIncludeSubtree } from "../../copySettings";
import { blockSubtreeMarkdown } from "./serialize";
import { pushToast } from "../../toasts";
import { activeDocument, queryAllWindows, requestFrame } from "../../windowRealm";

installClearOutlineSelection(() => clearSelection());
// I-20: selected ids name blocks of the bound graph; a same-id block in the next
// graph must not inherit the selection (Delete would remove it).
clearOnBindingInvalidated(() => clearSelection());

let activeSelectionScope: OutlineScope | null = null;

/** Visible order to resolve a block SELECTION against. The journals feed lives in
 *  visibleData(); a routed single page is loaded via ensurePageLoaded and is NOT in
 *  doc.feed, so its blocks aren't in visibleOrder() — fall back to that block's own
 *  page order, mirroring prevVisible/nextVisible. Without this, block-select (Esc,
 *  Arrow, Shift+Arrow) is dead on any routed page / reference / embed. */
function selectionOrder(id: string | null, scope: OutlineScope | null = activeSelectionScope): string[] {
  if (!id) return [];
  if (scope) return scopedVisibleOrder(scope);
  if (visibleData().index.has(id)) return visibleOrder();
  const page = doc.byId[id]?.page;
  return page ? pageVisibleOrder(page) : [];
}

// ---------------------------------------------------------------------------
// Multi-block selection (Escape from editing; Shift+Arrows extend) + ops
// ---------------------------------------------------------------------------

const [selAnchor, setSelAnchor] = createSignal<string | null>(null);
const [selFocus, setSelFocus] = createSignal<string | null>(null);

/** True when `ancestor` is a strict ancestor of `id` in the block tree. */
function isAncestorId(ancestor: string, id: string): boolean {
  let p = doc.byId[id]?.parent ?? null;
  while (p !== null) {
    if (p === ancestor) return true;
    p = doc.byId[p]?.parent ?? null;
  }
  return false;
}

/** Index of the last visible descendant of order[headIdx] within `order`.
 *  (A subtree occupies one contiguous DFS slice of the visible order.) */
function subtreeEndIndex(order: string[], headIdx: number): number {
  const head = order[headIdx];
  let end = headIdx;
  for (let k = headIdx + 1; k < order.length && isAncestorId(head, order[k]); k++) end = k;
  return end;
}

/** The selected block ids in visible order: the anchor..focus slice, where a
 *  reverse (Shift+Up) slice also completes the visible subtree of every member
 *  that is an ancestor of the anchor, so a selection never holds a parent
 *  without its later children (GH #262). Cost O(slice · depth). */
export function selectedIds(): string[] {
  const a = selAnchor();
  const f = selFocus();
  if (!a || !f) return [];
  const order = selectionOrder(a);
  const i = order.indexOf(a);
  const j = order.indexOf(f);
  if (i < 0 || j < 0) return [];
  const lo = Math.min(i, j);
  let hi = Math.max(i, j);
  // GH #262 Shift+Up: extending UP from inside a subtree onto its parent left
  // the slice [parent … anchor], omitting the parent's later children — a
  // partial-subtree selection that copy/cut/move received as "parent without
  // its children". (Shift+Down is the semantic reverse and never produces it:
  // a parent's children all follow it in visible order.) In a reverse slice,
  // any member that is an ancestor of the anchor may hold a partial subtree
  // inside the slice; complete its visible subtree.
  if (i > j) {
    for (let k = lo; k <= hi; k++) {
      if (isAncestorId(order[k], a)) {
        const end = subtreeEndIndex(order, k);
        if (end > hi) hi = end;
      }
    }
  }
  return order.slice(lo, hi + 1);
}
// Memoized set of selected ids. `isSelected` is read in the render of EVERY
// block (Block.tsx classList), and selectedIds() rebuilds visibleOrder() each
// call — so without this, a selection over N visible blocks costs O(N²). The
// memo recomputes only when the anchor/focus or the visible tree changes.
const selectedSet = createRoot(() => createMemo(() => new Set(selectedIds())));
export function isSelected(id: string): boolean {
  return selectedSet().has(id);
}
// Hierarchical Ctrl/Cmd+A (GH #262): the ancestor whose visible subtree the
// current select-all sequence covers. Any other selection mutation resets it.
let selectAllHead: string | null = null;

export function selectBlock(id: string, scope: OutlineScope | null = null) {
  endEdit("select-block");
  notifyOutlineSelectionStarted(id);
  activeSelectionScope = scope;
  selectAllHead = null;
  setSelAnchor(id);
  setSelFocus(id);
}
export function clearSelection() {
  setSelAnchor(null);
  setSelFocus(null);
  activeSelectionScope = null;
  selectAllHead = null;
}
/** Extend the current block selection's focus to `id` (mouse-drag / shift-click).
 *  Starts a fresh selection anchored at `id` if none is active. */
export function extendSelectionTo(id: string, scope: OutlineScope | null = activeSelectionScope) {
  notifyOutlineSelectionStarted(id);
  if (selAnchor() === null) {
    activeSelectionScope = scope;
    setSelAnchor(id);
  }
  if (activeSelectionScope && !scopedVisibleOrder(activeSelectionScope).includes(id)) return;
  selectAllHead = null;
  setSelFocus(id);
}
export function hasSelection(): boolean {
  return selAnchor() !== null;
}
export function moveSelection(dir: 1 | -1, extend: boolean) {
  const f = selFocus();
  if (!f) return;
  const order = selectionOrder(f);
  const i = order.indexOf(f);
  const ni = i + dir;
  if (ni < 0 || ni >= order.length) return;
  const next = order[ni];
  selectAllHead = null;
  setSelFocus(next);
  if (!extend) setSelAnchor(next);
  scrollBlockRowIntoView(next);
}

/** Ctrl/Cmd+A while editing a block, with the block's text already fully
 *  selected: escalate to a block selection covering the block's whole visible
 *  subtree, and mark it the head of the hierarchy ladder (GH #262). */
export function selectBlockSubtree(id: string, scope: OutlineScope | null = null) {
  selectBlock(id, scope);
  const order = selectionOrder(id);
  const idx = order.indexOf(id);
  if (idx < 0) return;
  setSelFocus(order[subtreeEndIndex(order, idx)]);
  selectAllHead = id;
}

/** Repeated Ctrl/Cmd+A in block-selection mode: widen the selection one
 *  ancestor level at a time — subtree, parent subtree, …, the whole visible
 *  outline, where it stays (idempotent) — or start the ladder at the current
 *  selection's anchor when no select-all sequence is in progress (GH #262). */
export function expandBlockSelection() {
  const a = selAnchor();
  const f = selFocus();
  if (!a || !f) return;
  const order = selectionOrder(a);
  if (order.length < 2) return;
  // Whole-outline selection is the top of the ladder; further presses no-op.
  if (a === order[0] && f === order[order.length - 1]) return;
  let head = selectAllHead;
  if (head === null || !order.includes(head)) head = a;
  let idx = order.indexOf(head);
  if (idx < 0) return;
  // A head whose subtree is fully inside the current selection (exactly or
  // because the user extended past it) is covered: climb from there.
  if (a === head && order.indexOf(f) >= subtreeEndIndex(order, idx)) {
    // This subtree is already fully selected: climb to its parent, or to the
    // whole outline when the head is already a root (within the active scope).
    const parent = doc.byId[head]?.parent ?? null;
    if (parent !== null && order.includes(parent)) {
      head = parent;
      idx = order.indexOf(head);
    } else {
      setSelAnchor(order[0]);
      setSelFocus(order[order.length - 1]);
      selectAllHead = null;
      return;
    }
  }
  setSelAnchor(head);
  setSelFocus(order[subtreeEndIndex(order, idx)]);
  selectAllHead = head;
}

/** Cycle every non-empty block in the active selection as one document
 * transaction. Each block advances from its own current marker, so a mixed
 * selection stays mixed (plain -> open, open -> active, active -> done). The
 * operation is all-or-nothing across read-only pages and preserves the visual
 * selection for repeated cycling. */
export function cycleSelectionTasks(): boolean {
  const ids = selectedIds().filter((id) => !!doc.byId[id]?.raw.trim());
  if (!ids.length || ids.some((id) => !blockWritable(id))) return false;

  const pages = [...new Set(ids.map((id) => doc.byId[id].page))];
  pushUndo("cycle-task-sel", pages);
  setDoc(
    produce((state) => {
      for (const id of ids) {
        const node = state.byId[id];
        if (!node) continue;
        // Match the existing editor command exactly: marker cycling handles
        // repeaters, while checkbox/marker-chip transitions own time tracking.
        node.raw = cycleMarkerSmart(node.raw, workflow(), formatForBlock(id)).raw;
      }
    })
  );
  if (pages.length > 1) void persistTogether(pages, "save-block");
  else for (const page of pages) markDirty(page, "save-block");
  return true;
}

/** Keep the active end of a keyboard selection on screen: as the user holds
 *  Arrow / Shift+Arrow past the top or bottom edge, reveal the newly-focused
 *  block. Targets the block's own row (`.block-main`), not the whole `.ls-block`
 *  (which spans its children and could be taller than the viewport), and uses
 *  `block: "nearest"` so it's a no-op while the row is already visible — it only
 *  scrolls when the row crosses an edge, and never recenters mid-page. Run on the
 *  next frame so the focus class is on the DOM before we measure. */
function scrollBlockRowIntoView(id: string) {
  // No-op under the test/headless runtime (no rAF/DOM); only the real webview scrolls.
  if (typeof requestAnimationFrame !== "function" || typeof document === "undefined") return;
  requestFrame(undefined, () => {
    const sel = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id;
    revealOutlineBlock(id);
    const row = activeDocument().querySelector(`.ls-block[data-block-id="${sel}"] > .block-main`)
      ?? queryAllWindows(`.ls-block[data-block-id="${sel}"] > .block-main`);
    row?.scrollIntoView({ block: "nearest" });
  });
}

/** Top-level selected blocks (exclude those whose parent is also selected). */
export function topSelected(): string[] {
  const ids = selectedIds();
  const set = new Set(ids);
  return ids.filter((id) => {
    const p = doc.byId[id]?.parent;
    return !(p && set.has(p));
  });
}

function selectionRemovalSurvivor(): string | null {
  const selected = selectedIds();
  const first = selected[0];
  const last = selected.at(-1);
  if (!first || !last) return null;
  return nextVisible(last) ?? prevVisible(first) ?? doc.byId[first]?.parent ?? null;
}

function reselectSurvivingBlock(id: string | null) {
  if (id && doc.byId[id]) selectBlock(id);
  else clearSelection();
}

/** The assumptions the old per-root move loop made, confirmed BEFORE the one-shot
 *  mutation: a stale or malformed tree stays a guarded no-op instead of committing
 *  a prefix of the selection (master b3fc9c813). */
function canBatchMoveSelectionRoots(ids: readonly string[], destPage: string, newParent: string | null): boolean {
  if (newParent !== null) {
    const parent = doc.byId[newParent];
    if (!parent || !blockWritable(newParent) || parent.page !== destPage) return false;
  }
  for (const id of ids) {
    const node = doc.byId[id];
    if (!node || !blockWritable(id) || node.page !== destPage || id === newParent) return false;
    if (!rootsOf(id).includes(id)) return false;
    // Never make a block its own ancestor; fail closed on a parent cycle.
    const seen = new Set<string>();
    let cursor = newParent;
    while (cursor !== null) {
      if (cursor === id || seen.has(cursor)) return false;
      seen.add(cursor);
      const ancestor = doc.byId[cursor];
      if (!ancestor) return false;
      cursor = ancestor.parent;
    }
    if (!existingSubtreeFits(id, newParent)) return false;
  }
  return true;
}

/** Remove every selected root from its own sibling array, then put them at one
 *  destination in document order: ONE publication and ONE dirty mark for the
 *  command (indent/outdent of a selection is one editor command, not N drags). */
function moveSelectionRootsInOneMutation(
  ids: readonly string[],
  destPage: string,
  destinationParent: string | null,
  destinationIndex: (state: DocState) => number,
  expandParent: string | null = null,
) {
  setDoc(
    produce((state) => {
      const siblingsFor = (id: string): string[] | null => {
        const node = state.byId[id];
        if (!node) return null;
        if (node.parent === null) return state.pages.find((page) => page.name === node.page)?.roots ?? null;
        return state.byId[node.parent]?.children ?? null;
      };
      // Check every removal before changing any array.
      if (ids.some((id) => (siblingsFor(id)?.indexOf(id) ?? -1) < 0)) return;
      // Re-read each index while removing: selected roots may share an array.
      for (const id of ids) {
        const siblings = siblingsFor(id)!;
        siblings.splice(siblings.indexOf(id), 1);
      }
      const destination = destinationParent === null
        ? state.pages.find((page) => page.name === destPage)?.roots
        : state.byId[destinationParent]?.children;
      if (!destination) return;
      const at = destinationIndex(state);
      if (at < 0) return;
      for (const id of ids) state.byId[id].parent = destinationParent;
      destination.splice(Math.min(at, destination.length), 0, ...ids);
      if (expandParent !== null) {
        const target = state.byId[expandParent];
        if (!target) return;
        // One raw rewrite in the same publication, not writeCollapsed() after the move.
        target.raw = rawWithCollapsed(target.raw, false, formatForBlock(expandParent));
        target.collapsed = false;
      }
    })
  );
  if (expandParent !== null) bumpCollapseEpochs([expandParent]);
  markDirty(destPage, "move-blocks");
}

export function indentSelection() {
  const ids = topSelected();
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
  const first = ids[0];
  const sibs = rootsOf(first);
  const fi = sibs.indexOf(first);
  if (fi <= 0) return;
  const newParent = sibs[fi - 1];
  if (activeSelectionScope && !scopedVisibleOrder(activeSelectionScope).includes(newParent)) return;
  // Structural indent is single-page ONLY. The target (newParent) is on first's
  // page; moving a block from another feed day under it would be a cross-page
  // structural move (removal-before-add hazard) — and indenting under a different
  // day's block is nonsensical anyway. So move only the selected blocks that are
  // already on the target page.
  const destPage = doc.byId[newParent].page;
  const same = ids.filter((id) => doc.byId[id]?.page === destPage);
  if (!same.length) return;
  if (same.some((id) => !existingSubtreeFits(id, newParent))) {
    pushToast("Outline is too deep to indent", "error");
    return;
  }
  if (!canBatchMoveSelectionRoots(same, destPage, newParent)) return;
  pushUndo("indent-sel", [destPage]);
  moveSelectionRootsInOneMutation(same, destPage, newParent, (state) => state.byId[newParent].children.length, newParent);
}

export function outdentSelection() {
  const ids = topSelected();
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
  const parentId = doc.byId[ids[0]].parent;
  if (parentId === null) return;
  if (activeSelectionScope?.forceExpandedRoot === parentId) return;
  const grand = doc.byId[parentId].parent;
  // Single-page only (see indentSelection): outdent moves blocks to `grand`, on
  // ids[0]'s page — so restrict to the blocks already on that page.
  const destPage = doc.byId[parentId].page;
  const same = ids.filter((id) => doc.byId[id]?.page === destPage);
  if (!same.length || !canBatchMoveSelectionRoots(same, destPage, grand)) return;
  if (!rootsOf(parentId).includes(parentId)) return;
  pushUndo("outdent-sel", [destPage]);
  moveSelectionRootsInOneMutation(same, destPage, grand, (state) => {
    const siblings = grand === null ? state.pages.find((page) => page.name === destPage)?.roots : state.byId[grand]?.children;
    const parentIndex = siblings?.indexOf(parentId) ?? -1;
    return parentIndex < 0 ? -1 : parentIndex + 1;
  });
}

export function deleteSelection() {
  const survivor = selectionRemovalSurvivor();
  const ids = topSelected();
  if (!ids.length || ids.some((id) => !blockWritable(id))) return;
  const pages = new Set<string>();
  for (const id of ids) {
    const n = doc.byId[id];
    if (n) pages.add(n.page);
  }
  pushUndo("delete-sel", [...pages]);
  // One produce for the whole selection — deleting each block separately fires a
  // reactive update per block (15 reflows for 15 bullets); batching collapses it
  // to a single update so the cut feels instant.
  setDoc(
    produce((s) => {
      for (const id of ids) {
        const node = s.byId[id];
        if (!node) continue;
        pages.add(node.page);
        const arr =
          node.parent === null
            ? s.pages[s.pages.findIndex((p) => p.name === node.page)].roots
            : s.byId[node.parent].children;
        const ix = arr.indexOf(id);
        if (ix >= 0) arr.splice(ix, 1);
        removeSubtree(s, id);
      }
    })
  );
  const ed = editingId();
  if (ed && !doc.byId[ed]) endEdit("delete-selection");
  if (pages.size > 1) void persistTogether(pages, "delete-blocks");
  else for (const p of pages) markDirty(p, "delete-blocks");
  reselectSurvivingBlock(survivor);
}

/** Public clipboard text for the selected roots; O(visible-order resolution + selected subtree bytes).
 * Defaults to the copy preference; cuts request complete subtrees so their
 * public clipboard flavor includes everything deletion removes. */
export function selectionMarkdown(includeSubtree = copyIncludeSubtree()): string {
  // Clipboard → always strip id:: (OG parity). collapsed:: and whole-subtree vs
  // selected-only are user-configurable (see copySettings): OG copies the full
  // sub-tree of a selected parent; Tine's default copies only the selected blocks.
  const stripCollapsed = copyStripCollapsed();
  const onlySel = includeSubtree ? undefined : new Set(selectedIds());
  return topSelected()
    .map((id) => blockSubtreeMarkdown(id, 0, true, stripCollapsed, onlySel))
    .join("\n");
}

/** Apply a context heading command to the active selection, falling back to the
 * pointer block only when no selection is active (master 6eea5b70c, GH #240).
 * The preflight makes a mixed writable/read-only selection an exact no-op. */
export function setSelectionHeading(pointerId: string, state: HeadingState): boolean {
  const selected = selectedIds();
  const ids = selected.length ? selected : [pointerId];
  if (!ids.length || ids.some((id) => !blockWritable(id))) return false;
  const changes = ids
    .map((id) => ({ id, page: doc.byId[id].page, raw: rawWithHeading(doc.byId[id].raw, formatForBlock(id), state) }))
    .filter((change) => change.raw !== doc.byId[change.id].raw);
  if (!changes.length) return true;
  const pages = [...new Set(changes.map((change) => change.page))];
  pushUndo("heading-selection", pages);
  setDoc(produce((stateDoc) => {
    for (const change of changes) stateDoc.byId[change.id].raw = change.raw;
  }));
  if (pages.length > 1) void persistTogether(pages, "save-block");
  else for (const page of pages) markDirty(page, "save-block");
  return true;
}
