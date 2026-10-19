// The pane's tab tree (GH #704 v2, DESIGN 2026-10-vertical-tabs). One owner:
// the pane router keeps its tabs as ONE array, and that array is the tree in
// depth-first order, so the strip, the ∨ overview and Ctrl+PgDn read the same
// flat order as before and nothing keeps a second order.
//
//   T1  every tab's descendants follow it contiguously (a tab's `parent` is an
//       earlier tab whose subtree has not ended yet);
//   T2  pinned tabs are roots and are never parents; they stay first.
//
// Every structural edit is a pure function here that ends in `settleTabTree`,
// which re-establishes both invariants for any input (a stale parent becomes a
// root, never a refused list). Cost: O(tabs × depth) per edit, no allocation
// beyond the new array.

export interface TreeTab {
  id: string;
  pinned: boolean;
  /** A tab id in the same pane, or null for a root. */
  parent: string | null;
  /** Hides this tab's descendants in the sidebar list only. */
  collapsed: boolean;
}

/** Stable partition: all pinned tabs first (in their relative order), then the
 *  unpinned ones. Pinned tabs always sit to the left of the strip (matches the
 *  OG plugin), so a pinned tab can't be visually stranded among unpinned ones. */
export function partitionPinned<T extends { pinned: boolean }>(list: readonly T[]): T[] {
  return [...list.filter((t) => t.pinned), ...list.filter((t) => !t.pinned)];
}

/** Re-establish T1/T2 over a list already in the intended order. A parent that
 *  is missing, later, pinned, or whose subtree already ended is dropped (the tab
 *  becomes a root where it stands); `collapsed` is cleared on a tab with no
 *  children, so a list without nesting carries no tree state at all. Returns
 *  the input array itself when nothing changed. */
export function normalizeTabTree<T extends TreeTab>(list: readonly T[]): T[] {
  const parents: (string | null)[] = [];
  const stack: string[] = []; // ancestor chain of the previous tab, itself included
  for (const t of list) {
    let parent = t.parent;
    if (t.pinned || parent === null) {
      parent = null;
      stack.length = 0;
    } else {
      const at = stack.lastIndexOf(parent);
      if (at < 0) {
        parent = null;
        stack.length = 0;
      } else stack.length = at + 1;
    }
    parents.push(parent);
    if (!t.pinned) stack.push(t.id);
  }
  const hasChildren = new Set(parents.filter((p): p is string => p !== null));
  let changed = false;
  const out = list.map((t, i) => {
    const parent = parents[i];
    const collapsed = t.collapsed && hasChildren.has(t.id);
    if (parent === t.parent && collapsed === t.collapsed) return t;
    changed = true;
    return { ...t, parent, collapsed };
  });
  return changed ? out : (list as T[]);
}

/** Pinned first, then T1/T2. Every structural edit ends here. */
export function settleTabTree<T extends TreeTab>(list: readonly T[]): T[] {
  return normalizeTabTree(partitionPinned(list));
}

/** Depth of every tab of a settled list (0 for a root). */
export function tabDepths(list: readonly TreeTab[]): number[] {
  const depths: number[] = [];
  const stack: string[] = [];
  for (const t of list) {
    if (t.parent === null) stack.length = 0;
    else {
      const at = stack.lastIndexOf(t.parent);
      stack.length = at < 0 ? 0 : at + 1;
    }
    depths.push(stack.length);
    if (!t.pinned) stack.push(t.id);
  }
  return depths;
}

/** One past the last descendant of the tab at `index` (T1 makes it contiguous). */
export function subtreeEnd(list: readonly TreeTab[], index: number, depths = tabDepths(list)): number {
  let end = index + 1;
  while (end < list.length && depths[end] > depths[index]) end++;
  return end;
}

/** Remove one tab; its children move up one level, in place (#704: closing a
 *  parent never closes its children). */
export function removeTabPromoting<T extends TreeTab>(list: readonly T[], id: string): T[] {
  const gone = list.find((t) => t.id === id);
  if (!gone) return list as T[];
  return settleTabTree(list
    .filter((t) => t.id !== id)
    .map((t) => (t.parent === id ? { ...t, parent: gone.parent } : t)));
}

/** Where a subtree may land in front of `rest[slot]`: the depth range the
 *  outliner rule allows (at most one under the row above, at least the depth
 *  of the row below; never under a pinned tab) and the parent at each depth. */
function slotRange(rest: readonly TreeTab[], slot: number) {
  const depths = tabDepths(rest);
  const above = rest[slot - 1];
  const below = rest[slot];
  const max = above && !above.pinned ? depths[slot - 1] + 1 : 0;
  const min = below && !below.pinned ? Math.min(depths[slot], max) : 0;
  // chain[d] = the tab at depth d on the path to `above` (the parent at depth d+1).
  const chain: string[] = [];
  if (above && !above.pinned) {
    for (let i = slot - 1, d = depths[slot - 1]; i >= 0 && d >= 0; i--) {
      if (depths[i] === d) { chain[d] = rest[i].id; d--; }
    }
  }
  return { min, max, parentAt: (depth: number) => (depth === 0 ? null : chain[depth - 1] ?? null) };
}

/** Move the tab `id` together with its subtree so it lands right before
 *  `beforeId` (null: at the end), measured on the list with the subtree lifted
 *  out. `depth` is the asked depth (clamped to what the slot allows); "keep"
 *  keeps the current parent when the slot allows it, else the shallowest depth
 *  (the flat strip's rule: a strip drag never nests anything by itself).
 *  Nesting into a collapsed tab expands it (and its ancestors), or the moved
 *  tab would vanish from the sidebar. */
export function moveTabSubtree<T extends TreeTab>(
  list: readonly T[],
  id: string,
  beforeId: string | null,
  depth: number | "keep",
): T[] {
  const from = list.findIndex((t) => t.id === id);
  if (from < 0) return list as T[];
  const depths = tabDepths(list);
  const end = subtreeEnd(list, from, depths);
  const block = list.slice(from, end);
  const rest = [...list.slice(0, from), ...list.slice(end)];
  let slot: number;
  if (beforeId === null) slot = rest.length;
  else if (block.some((t) => t.id === beforeId)) slot = from;
  else slot = rest.findIndex((t) => t.id === beforeId);
  if (slot < 0) return list as T[];
  const moved = block[0];
  let parent: string | null = null;
  if (!moved.pinned) {
    const range = slotRange(rest, slot);
    let d: number;
    if (depth === "keep") {
      d = range.min;
      for (let k = range.min; k <= range.max; k++) {
        if (range.parentAt(k) === moved.parent) { d = k; break; }
      }
    } else d = Math.max(range.min, Math.min(Math.round(depth), range.max));
    parent = range.parentAt(d);
  }
  const placed = [{ ...moved, parent }, ...block.slice(1)] as T[];
  // The new parent and every ancestor of it open, so the moved tab is visible.
  const opened = new Set<string>();
  const byId = new Map(rest.map((t) => [t.id, t] as const));
  for (let a = parent; a !== null && !opened.has(a); a = byId.get(a)?.parent ?? null) opened.add(a);
  const out = [...rest.slice(0, slot), ...placed, ...rest.slice(slot)]
    .map((t) => (opened.has(t.id) && t.collapsed ? { ...t, collapsed: false } : t));
  return settleTabTree(out);
}

/** Move a nested tab (with its subtree) one level out: it becomes its
 *  parent's next sibling, right after the parent's subtree. No-op on a root. */
export function moveTabOutOfGroup<T extends TreeTab>(list: readonly T[], id: string): T[] {
  const tab = list.find((t) => t.id === id);
  if (!tab || tab.parent === null) return list as T[];
  const parentIndex = list.findIndex((t) => t.id === tab.parent);
  if (parentIndex < 0) return list as T[];
  const depths = tabDepths(list);
  const end = subtreeEnd(list, parentIndex, depths);
  const before = list[end]?.id ?? null;
  return moveTabSubtree(list, id, before, depths[parentIndex]);
}

/** Collapse or expand a tab; collapsing a tab with no children is a no-op. */
export function setTabCollapsed<T extends TreeTab>(list: readonly T[], id: string, collapsed: boolean): T[] {
  const tab = list.find((t) => t.id === id);
  if (!tab || tab.collapsed === collapsed) return list as T[];
  if (collapsed && !list.some((t) => t.parent === id)) return list as T[];
  return list.map((t) => (t.id === id ? { ...t, collapsed } : t));
}

/** A slot at or after `index` where a root may be inserted without splitting
 *  a group (an arriving tab is a root). */
export function rootSlot(list: readonly TreeTab[], index: number): number {
  const depths = tabDepths(list);
  let slot = Math.min(Math.max(0, index | 0), list.length);
  while (slot < list.length && depths[slot] > 0) slot++;
  return slot;
}
