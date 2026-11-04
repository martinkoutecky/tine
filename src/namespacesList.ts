import type { NsNode } from "./components/Namespace";

// The full Namespaces view's list logic (GH #335): which rows, in what order, with
// what counts. Pure functions over the sidebar's namespace tree
// (`buildNamespaceTree` over the shared page-name inventory); nothing here reads the
// backend, so expanding, collapsing and filtering cost no IPC and a row costs no call.
//
// The tree is flattened ONCE per page-set change into a preorder `NsEntry[]`, each
// entry knowing where its subtree ends. Expand/collapse then walks only the visible
// rows (a collapsed subtree is skipped in one step), and the filter is one lowercase
// substring test per entry, the same shape as the All pages filter.

export interface NsEntry {
  node: NsNode;
  depth: number;
  /** Index of the parent entry, or -1 for a top-level namespace. */
  parent: number;
  /** Index one past this entry's last descendant, so `end - index - 1` entries lie below it. */
  end: number;
  /** Pages strictly below this entry (every node is a listed name, namespace
   *  parents included): its size. Zero for a leaf. */
  count: number;
  /** `node.full` lowercased, for the filter. */
  lower: string;
}

/** Preorder entries of `tree` without recursion, so an imported name with thousands of
 *  segments cannot overflow the stack. O(entries). */
export function indexNamespaces(tree: readonly NsNode[]): NsEntry[] {
  const entries: NsEntry[] = [];
  const pending: { node: NsNode; depth: number; parent: number }[] = [];
  for (let i = tree.length - 1; i >= 0; i--) pending.push({ node: tree[i], depth: 0, parent: -1 });
  // Open ancestors of the entry being added, deepest last.
  const open: number[] = [];
  while (pending.length) {
    const { node, depth, parent } = pending.pop()!;
    const index = entries.length;
    while (open.length && entries[open[open.length - 1]].depth >= depth) {
      const closed = open.pop()!;
      entries[closed].end = index;
    }
    entries.push({ node, depth, parent, end: index + 1, count: 0, lower: node.full.toLowerCase() });
    open.push(index);
    for (let i = node.children.length - 1; i >= 0; i--) pending.push({ node: node.children[i], depth: depth + 1, parent: index });
  }
  for (const closed of open) entries[closed].end = entries.length;

  for (let i = 0; i < entries.length; i++) entries[i].count = entries[i].end - i - 1;
  return entries;
}

/** Pages in the whole tree (every entry is a listed name), and the entries that have something below them. */
export function namespaceTotals(entries: readonly NsEntry[]): { pages: number; namespaces: number } {
  let namespaces = 0;
  for (const entry of entries) if (entry.node.children.length) namespaces++;
  return { pages: entries.length, namespaces };
}

/** What is open when the view first appears: the top-level namespaces, as in the sidebar tree. */
export function defaultExpanded(entries: readonly NsEntry[]): Set<string> {
  const open = new Set<string>();
  for (const entry of entries) if (entry.depth === 0 && entry.node.children.length) open.add(entry.node.full);
  return open;
}

/** Every entry that has children: "Expand all". */
export function allExpanded(entries: readonly NsEntry[]): Set<string> {
  const open = new Set<string>();
  for (const entry of entries) if (entry.node.children.length) open.add(entry.node.full);
  return open;
}

/** Indices of the rows on screen with `expanded` open: the walk jumps over the subtree of
 *  every collapsed entry. O(visible rows + collapsed entries met). */
export function visibleEntries(entries: readonly NsEntry[], expanded: ReadonlySet<string>): number[] {
  const rows: number[] = [];
  let i = 0;
  while (i < entries.length) {
    rows.push(i);
    const entry = entries[i];
    i = entry.node.children.length && !expanded.has(entry.node.full) ? entry.end : i + 1;
  }
  return rows;
}

/** Indices of the entries whose full path contains `filter` (case-insensitive), in tree
 *  order, so a parent precedes its children. A blank filter matches nothing here: the
 *  caller shows the tree instead. O(entries). */
export function matchingEntries(entries: readonly NsEntry[], filter: string): number[] {
  const needle = filter.trim().toLowerCase();
  if (!needle) return [];
  const rows: number[] = [];
  for (let i = 0; i < entries.length; i++) if (entries[i].lower.includes(needle)) rows.push(i);
  return rows;
}

/** The path above a row's own segment, with its trailing slash: `a/b/` for `a/b/c`. */
export function pathPrefix(entry: NsEntry): string {
  return entry.node.full.slice(0, entry.node.full.length - entry.node.seg.length);
}
