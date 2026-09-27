import { OutlineScope, scopedVisibleOrder, visibleData, visibleOrder, pageVisibleOrder, nextVisible, prevVisible, rootsOf, indexInSiblings } from "../tree";
import { doc, setDoc } from "../model";
import { createSignal, createRoot, createMemo } from "solid-js";
import { endEdit, editingId } from "../../editorController";
import { installClearOutlineSelection, notifyOutlineSelectionStarted } from "../../modeHooks";
import { blockWritable, writeCollapsed } from "./properties";
import { pushUndo } from "../history";
import { produce } from "solid-js/store";
import { cycleMarkerSmart } from "../../editor/repeat";
import { workflow } from "../../ui";
import { markDirty } from "../save/engine";
import { moveBlockInternal } from "./moves";
import { copyStripCollapsed, copyIncludeSubtree } from "../../copySettings";
import { blockSubtreeMarkdown } from "./serialize";

installClearOutlineSelection(() => clearSelection());

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

export function selectedIds(): string[] {
  const a = selAnchor();
  const f = selFocus();
  if (!a || !f) return [];
  const order = selectionOrder(a);
  let i = order.indexOf(a);
  let j = order.indexOf(f);
  if (i < 0 || j < 0) return [];
  if (i > j) [i, j] = [j, i];
  return order.slice(i, j + 1);
}
// Memoized set of selected ids. `isSelected` is read in the render of EVERY
// block (Block.tsx classList), and selectedIds() rebuilds visibleOrder() each
// call — so without this, a selection over N visible blocks costs O(N²). The
// memo recomputes only when the anchor/focus or the visible tree changes.
const selectedSet = createRoot(() => createMemo(() => new Set(selectedIds())));
export function isSelected(id: string): boolean {
  return selectedSet().has(id);
}
export function selectBlock(id: string, scope: OutlineScope | null = null) {
  endEdit("select-block");
  notifyOutlineSelectionStarted(id);
  activeSelectionScope = scope;
  setSelAnchor(id);
  setSelFocus(id);
}
export function clearSelection() {
  setSelAnchor(null);
  setSelFocus(null);
  activeSelectionScope = null;
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
  setSelFocus(next);
  if (!extend) setSelAnchor(next);
  scrollBlockRowIntoView(next);
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
        node.raw = cycleMarkerSmart(node.raw, workflow()).raw;
      }
    })
  );
  for (const page of pages) markDirty(page);
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
  requestAnimationFrame(() => {
    const sel = typeof CSS !== "undefined" && CSS.escape ? CSS.escape(id) : id;
    const row = document.querySelector(`.ls-block[data-block-id="${sel}"] > .block-main`);
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
  pushUndo("indent-sel", [destPage]);
  for (const id of same) moveBlockInternal(id, newParent, doc.byId[newParent].children.length);
  writeCollapsed(newParent, false);
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
  if (!same.length) return;
  pushUndo("outdent-sel", [destPage]);
  let after = parentId;
  for (const id of same) {
    moveBlockInternal(id, grand, indexInSiblings(after) + 1);
    after = id;
  }
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
        const rm = (bid: string) => {
          for (const c of s.byId[bid].children) rm(c);
          delete s.byId[bid];
        };
        rm(id);
      }
    })
  );
  const ed = editingId();
  if (ed && !doc.byId[ed]) endEdit("delete-selection");
  for (const p of pages) markDirty(p);
  reselectSurvivingBlock(survivor);
}

export function selectionMarkdown(): string {
  // Clipboard → always strip id:: (OG parity). collapsed:: and whole-subtree vs
  // selected-only are user-configurable (see copySettings): OG copies the full
  // sub-tree of a selected parent; Tine's default copies only the selected blocks.
  const stripCollapsed = copyStripCollapsed();
  const onlySel = copyIncludeSubtree() ? undefined : new Set(selectedIds());
  return topSelected()
    .map((id) => blockSubtreeMarkdown(id, 0, true, stripCollapsed, onlySel))
    .join("\n");
}
