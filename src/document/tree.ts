import { doc, blockIsOpaqueSheetView, mainPages, formatForBlock } from "./model";
import { createRoot, createMemo } from "solid-js";
import { isEditorEmpty, BUILTIN_HIDE } from "../editor/properties";
import { outlineMaxDepth } from "../editor/outline";
import { commentsFirst } from "./commentOrder";

// ---------------------------------------------------------------------------
// Tree helpers
// ---------------------------------------------------------------------------

export function rootsOf(id: string): string[] {
  const n = doc.byId[id];
  if (n.parent !== null) return doc.byId[n.parent].children;
  const p = doc.pages.find((x) => x.name === n.page);
  return p ? p.roots : [];
}

export function indexInSiblings(id: string): number {
  return rootsOf(id).indexOf(id);
}

/** Page-local preorder without DTO copies. Scopes omit missing nodes and can
 * override collapse; ordinary page/feed order retains missing root ids. O(visible nodes). */
function visitVisible(ids: readonly string[], visit: (id: string) => void, scope?: OutlineScope): void {
  for (const id of ids) {
    const node = doc.byId[id];
    if (!node && scope) continue;
    visit(id);
    if (!node) continue;
    const collapsed = scope?.collapsed?.(id, node.collapsed) ?? node.collapsed;
    if ((!collapsed || id === scope?.forceExpandedRoot) && node.children.length && !blockIsOpaqueSheetView(id)) {
      // Comments show under their block, before its other children (commentOrder.ts).
      visitVisible(commentsFirst(node.children), visit, scope);
    }
  }
}

/** Visible blocks in the MAIN view, in display order (drives editor arrow-nav),
 *  plus an id→index map. Memoized: it's recomputed only when the feed, a
 *  collapsed/children state, or a block's opaque-sheet answer changes — plain
 *  typing reruns only that block's node-scoped sheet memo, never this walk
 *  (`visibleOrderCost.test.ts`). Shared across the many callers in one tick.
 *  Scoped to the feed so navigation stays within the main content area, not
 *  satellite pages loaded for the sidebar/queries. */
export const visibleData = createRoot(() =>
  createMemo(() => {
    const order: string[] = [];
    const index = new Map<string, number>();
    const append = (id: string) => { index.set(id, order.length); order.push(id); };
    for (const p of mainPages()) visitVisible(p.roots, append);
    return { order, index };
  })
);
export function visibleOrder(): string[] {
  return visibleData().order;
}

// Visible (expanded) block order within a single page — the fallback for blocks
// that aren't part of the main routed view, e.g. the quick-capture scratch page,
// whose roots never appear in mainPages(). Without this, prevVisible/nextVisible
// (and therefore Backspace-merge and Up/Down nav) are dead in the capture window.
export function pageVisibleOrder(pageName: string, revealId?: string): string[] {
  const order: string[] = [];
  const page = doc.pages.find((p) => p.name === pageName);
  if (!page) return order;
  // GH #766: a secondary view can expose a child whose source ancestors are
  // folded. Reveal only its ancestor path in SOURCE order for structural
  // navigation; never use the arbitrary result list as a merge topology.
  const ancestors = new Set<string>();
  let parent = revealId ? doc.byId[revealId]?.parent : null;
  while (parent && !ancestors.has(parent)) {
    ancestors.add(parent);
    parent = doc.byId[parent]?.parent;
  }
  const scope = revealId ? {
    roots: page.roots,
    collapsed: (id: string, stored: boolean) => ancestors.has(id) ? false : stored,
  } : undefined;
  visitVisible(page.roots, (id) => order.push(id), scope);
  return order;
}

/** Model-only description of the outline currently rendered around a block.
 * Zoom uses a single root whose durable collapse is overridden for this view.
 *
 * Reference/query/embed groups render an ARBITRARY display list of roots
 * (backlink hits, query results) that is not the outline. Such a scope is
 * `navOnly`: arrow navigation and view-local selection read it, but structural
 * mutations (merges/indents/moves) must NOT treat the display list as the
 * outline — they fall back to page order instead (master GH #341). */
export interface OutlineScope {
  roots: string[];
  forceExpandedRoot?: string;
  /** A secondary surface's collapse contract, so the scoped visible order
   * mirrors what is rendered rather than the durable `node.collapsed` flags. */
  collapsed?: (id: string, stored: boolean) => boolean;
  navOnly?: boolean;
}

export function scopedVisibleOrder(scope: OutlineScope): string[] {
  const order: string[] = [];
  visitVisible(scope.roots, (id) => order.push(id), scope);
  return order;
}

/** The only trailing-block reuse candidate for a rendered outline boundary.
 * The caller must supply the actual page or zoom scope so journal days cannot
 * cross-select each other. A collapsed parent and an opaque Sheet host remain
 * visible terminal rows, but their storage children mean neither is a leaf. */
export function trailingVisibleEmptyLeaf(scope: OutlineScope): string | null {
  const id = scopedVisibleOrder(scope).at(-1);
  if (!id) return null;
  const node = doc.byId[id];
  if (!node || node.children.length !== 0) return null;
  const format = formatForBlock(id);
  return isEditorEmpty(node.raw, BUILTIN_HIDE, format) ? id : null;
}

export function prevVisible(id: string, scope: OutlineScope | null = null): string | null {
  if (scope) {
    const order = scopedVisibleOrder(scope);
    const i = order.indexOf(id);
    return i > 0 ? order[i - 1] : null;
  }
  const { order, index } = visibleData();
  const i = index.get(id);
  if (i !== undefined) return i > 0 ? order[i - 1] : null;
  const node = doc.byId[id];
  if (!node) return null;
  const ord = pageVisibleOrder(node.page, id);
  const j = ord.indexOf(id);
  return j > 0 ? ord[j - 1] : null;
}

export function nextVisible(id: string, scope: OutlineScope | null = null): string | null {
  if (scope) {
    const order = scopedVisibleOrder(scope);
    const i = order.indexOf(id);
    return i >= 0 && i < order.length - 1 ? order[i + 1] : null;
  }
  const { order, index } = visibleData();
  const i = index.get(id);
  if (i !== undefined) return i < order.length - 1 ? order[i + 1] : null;
  const node = doc.byId[id];
  if (!node) return null;
  const ord = pageVisibleOrder(node.page, id);
  const j = ord.indexOf(id);
  return j >= 0 && j < ord.length - 1 ? ord[j + 1] : null;
}

export function depthOf(id: string): number {
  let d = 0;
  let p = doc.byId[id]?.parent ?? null;
  while (p !== null) {
    d++;
    p = doc.byId[p].parent;
  }
  return d;
}

/** Check the deepest descendant after reparenting an existing subtree. */
export function existingSubtreeFits(id: string, newParent: string | null): boolean {
  if (!doc.byId[id]) return false;
  const firstDepth = newParent === null ? 0 : depthOf(newParent) + 1;
  const pending: Array<[string, number]> = [[id, firstDepth]];
  while (pending.length) {
    const [current, depth] = pending.pop()!;
    if (depth >= outlineMaxDepth()) return false;
    for (const child of doc.byId[current].children) pending.push([child, depth + 1]);
  }
  return true;
}
