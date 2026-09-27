import { doc, blockIsOpaqueSheetView, mainPages, formatForBlock } from "./model";
import { createRoot, createMemo } from "solid-js";
import { splitProps, isBuiltinHidden } from "../editor/properties";

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

/** Visible blocks in the MAIN view, in display order (drives editor arrow-nav),
 *  plus an id→index map. Memoized: it's recomputed only when the feed or a
 *  collapsed/children state changes (NOT on plain typing), and shared across the
 *  many callers in one tick. Scoped to the feed so navigation stays within the
 *  main content area, not satellite pages loaded for the sidebar/queries. */
export const visibleData = createRoot(() =>
  createMemo(() => {
    const order: string[] = [];
    const index = new Map<string, number>();
    const walk = (ids: readonly string[]) => {
      for (const id of ids) {
        index.set(id, order.length);
        order.push(id);
        const n = doc.byId[id];
        if (n && !n.collapsed && n.children.length && !blockIsOpaqueSheetView(id)) walk(n.children);
      }
    };
    for (const p of mainPages()) walk(p.roots);
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
export function pageVisibleOrder(pageName: string): string[] {
  const order: string[] = [];
  const page = doc.pages.find((p) => p.name === pageName);
  if (!page) return order;
  const walk = (ids: string[]) => {
    for (const id of ids) {
      order.push(id);
      const n = doc.byId[id];
      if (n && !n.collapsed && n.children.length && !blockIsOpaqueSheetView(id)) walk(n.children);
    }
  };
  walk(page.roots);
  return order;
}

/** Model-only description of the outline currently rendered around a block.
 * Zoom uses a single root whose durable collapse is overridden for this view. */
export interface OutlineScope {
  roots: string[];
  forceExpandedRoot?: string;
}

export function scopedVisibleOrder(scope: OutlineScope): string[] {
  const order: string[] = [];
  const walk = (ids: readonly string[]) => {
    for (const id of ids) {
      const node = doc.byId[id];
      if (!node) continue;
      order.push(id);
      const expanded = !node.collapsed || id === scope.forceExpandedRoot;
      if (expanded && node.children.length && !blockIsOpaqueSheetView(id)) walk(node.children);
    }
  };
  walk(scope.roots);
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
  return splitProps(node.raw, isBuiltinHidden, formatForBlock(id)).visible.trim() === "" ? id : null;
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
  const ord = pageVisibleOrder(node.page);
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
  const ord = pageVisibleOrder(node.page);
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
