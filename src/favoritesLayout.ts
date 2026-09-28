// The Favorites arrangement: nesting and order, stored as an ordinary page named
// by `:tine/favorites-page` (port of master 7cefc9a7c/7b162cb8d). A page, not a
// blob, so it merges and resolves conflicts like any page, and `[[links]]`
// follow renames. Membership stays in `config.edn :favorites` as the flat,
// Logseq-readable list, projected from this tree in pre-order.
//
//     tine/favorites:: true
//
//     - [[Alpha]]            <- a favorite: the bullet is exactly one link
//     - Work                 <- a label: any other text (not a page)
//     	- [[Beta]]            <- both nest, to any depth
//
// One node type. Every bullet round-trips verbatim; a blank bullet is dropped
// but its children are kept.
import type { BlockDto, PageKind } from "./types";
import { isJournalTitle } from "./journal";
import { pageIdentityKey } from "./pageIdentity";

export const FAVORITES_PAGE_PROPERTY = "tine/favorites";
export const DEFAULT_FAVORITES_PAGE = "Favorites";

export interface FavNode {
  /** The page a link-only bullet points at; `null` for a label. */
  target: string | null;
  raw: string;
  /** In-session kind of a favorite; on disk kind is re-derived from the name. */
  kind?: PageKind;
  collapsed?: boolean;
  children: FavNode[];
}
export type FavLayout = FavNode[];
export interface FavItem { name: string; kind: PageKind }
/** A drawn row: its node, its path (child indices from the root), its depth. */
export interface FavRow { path: number[]; node: FavNode; depth: number }

const LINK_ONLY = /^\s*\[\[([^\]]+)\]\]\s*$/;
export function linkOnlyTarget(raw: string): string | null {
  return LINK_ONLY.exec(raw)?.[1]?.trim() || null;
}
export const itemKind = (name: string): PageKind => (isJournalTitle(name) ? "journal" : "page");
export const favoriteNode = (name: string, kind?: PageKind): FavNode =>
  ({ target: name, raw: `[[${name}]]`, ...(kind && kind !== itemKind(name) ? { kind } : {}), children: [] });
export const labelNode = (name: string): FavNode => ({ target: null, raw: name, children: [] });

export function layoutFromBlocks(roots: readonly BlockDto[]): FavLayout {
  return roots.flatMap((block): FavNode[] => {
    const children = layoutFromBlocks(block.children);
    if (!block.raw.trim()) return children;
    // raw stays verbatim: it is written back byte for byte.
    return [{ target: linkOnlyTarget(block.raw), raw: block.raw, collapsed: block.collapsed || undefined, children }];
  });
}

/** Tab-indented bullets, as Tine writes every page. Also the structural
 *  identity used to tell an echo of our own write from a real edit. */
export function layoutToMarkdown(layout: FavLayout, depth = 0): string {
  const indent = "\t".repeat(depth);
  return layout.map((node) => `${indent}- ${node.raw}\n${node.collapsed ? `${indent}  collapsed:: true\n` : ""}${layoutToMarkdown(node.children, depth + 1)}`).join("");
}

/** The arrangement carries something `:favorites` cannot express. */
export const carriesArrangement = (layout: FavLayout) =>
  layout.some((node) => node.target === null || node.children.length > 0);

/** Flat membership in pre-order: what `:favorites` receives. */
export function layoutMembers(layout: FavLayout): FavItem[] {
  return layout.flatMap((node) => [
    ...(node.target ? [{ name: node.target, kind: node.kind ?? itemKind(node.target) }] : []),
    ...layoutMembers(node.children),
  ]);
}

/** Fold a membership list into the arrangement: a removed favorite's children
 *  take its place, a duplicate spelling is dropped, a new member is appended at
 *  the top level, labels are always kept. `key` is the membership identity. */
export function reconcileLayout(layout: FavLayout, membership: string[], key: (name: string) => string = pageIdentityKey): FavLayout {
  const wanted = new Map(membership.map((name) => [key(name), name] as const));
  const seen = new Set<string>();
  const keep = (nodes: FavNode[]): FavNode[] => nodes.flatMap((node) => {
    const children = keep(node.children);
    if (node.target === null) return [{ ...node, children }];
    const k = key(node.target);
    if (!wanted.has(k) || seen.has(k)) return children;
    seen.add(k);
    return [{ ...node, children }];
  });
  const next = keep(layout);
  for (const [k, name] of wanted) if (!seen.has(k)) next.push(favoriteNode(name));
  return next;
}

/** A label name no other label already uses ("Work", "Work 2", …). */
export function uniqueGroupName(layout: FavLayout, desired: string): string {
  const taken = new Set<string>();
  const walk = (nodes: FavNode[]): void => nodes.forEach((node) => {
    if (node.target === null) taken.add(pageIdentityKey(node.raw));
    walk(node.children);
  });
  walk(layout);
  let candidate = desired;
  for (let n = 2; taken.has(pageIdentityKey(candidate)); n += 1) candidate = `${desired} ${n}`;
  return candidate;
}

export function nodeAt(layout: FavLayout, path: number[]): FavNode | null {
  let found: FavNode | null = null;
  for (const index of path) {
    found = (found ? found.children : layout)[index] ?? null;
    if (!found) return null;
  }
  return found;
}

/** Replace the node at `path` with zero or more nodes, rebuilding only its
 *  ancestors. Every structural mutation goes through here. */
export function updateAt(layout: FavLayout, path: number[], update: (node: FavNode) => FavNode[]): FavLayout {
  const [head, ...rest] = path;
  return layout.flatMap((node, i) => i !== head ? [node]
    : rest.length ? [{ ...node, children: updateAt(node.children, rest, update) }] : update(node));
}

/** Insert as the `index`th child of `parent` (`[]` = top level). Dropping
 *  into a collapsed row expands it, or the moved row would vanish. */
function insertAt(layout: FavLayout, parent: number[], index: number, node: FavNode): FavLayout {
  if (!parent.length) {
    const next = [...layout];
    next.splice(Math.max(0, Math.min(index, next.length)), 0, node);
    return next;
  }
  return updateAt(layout, parent, (target) => [
    { ...target, collapsed: undefined, children: insertAt(target.children, [], index, node) },
  ]);
}

/** Drawn rows in pre-order, skipping what a collapsed row hides. The row's
 *  index is its `data-row-index`. */
export function visibleRows(layout: FavLayout, prefix: number[] = [], depth = 0): FavRow[] {
  return layout.flatMap((node, i) => {
    const path = [...prefix, i];
    return [{ path, node, depth }, ...(node.collapsed ? [] : visibleRows(node.children, path, depth + 1))];
  });
}

/** `path` is `root` or inside it. */
export const isWithin = (path: number[], root: number[]) =>
  path.length >= root.length && root.every((value, i) => path[i] === value);

/** Resolve a drop at `slot` (between visible rows) asking for `desiredDepth`
 *  into (parent, index). The outliner rule clamps the depth: at most one level
 *  under the row above, at least the depth of the row below. */
export function resolveDrop(rows: FavRow[], slot: number, desiredDepth: number): { parent: number[]; index: number; depth: number } {
  const above = rows[slot - 1];
  const below = rows[slot];
  const max = above ? above.depth + 1 : 0;
  const min = below ? Math.min(below.depth, max) : 0;
  const depth = Math.max(min, Math.min(desiredDepth, max));
  for (let i = slot - 1; depth > 0 && i >= 0; i -= 1) {
    if (rows[i].depth !== depth - 1) continue;
    const parent = rows[i].path;
    const index = rows.slice(i + 1, slot).filter((row) => row.path.length === parent.length + 1 && isWithin(row.path, parent)).length;
    return { parent, index, depth };
  }
  return { parent: [], index: rows.slice(0, slot).filter((row) => row.depth === 0).length, depth: 0 };
}

/** Move the node at `from` under `parent` at `index`. `from` and `parent` are
 *  paths before the move; `index` counts `parent`'s children after the node is
 *  lifted out (what `resolveDrop` over the remaining rows produces). */
export function moveNode(layout: FavLayout, from: number[], parent: number[], index: number): FavLayout {
  const node = nodeAt(layout, from);
  if (!node || !from.length || isWithin(parent, from)) return layout;
  const at = from.length - 1;
  const shifted = parent.length > at && isWithin(parent, from.slice(0, at)) && parent[at] > from[at];
  const adjusted = shifted ? parent.map((value, i) => (i === at ? value - 1 : value)) : parent;
  return insertAt(updateAt(layout, from, () => []), adjusted, index, node);
}
