import { createMemo } from "solid-js";
import { blockIsOpaqueSheetView, node } from "../document";
import { authorOf, quoteSelectorOf } from "../comments";
import { authorKey } from "../commentIdentity";
import type { CommentIndex } from "../commentIndex";
import { facetsOf, parseBody } from "../render/facets";
import type { Block, Inline, ListItem } from "../render/ast";

/** What a row's renderer knows, in O(1), about the rows drawn next to it. `prev`/`next` are the
 * neighbours in the row's own list. The first row of a block's child list also gets `head`, the parent
 * above it; the last gets `after`, the row drawn below the parent's whole subtree. A `results` list
 * (references, queries, embeds) is a display list, not an outline: its rows are not reading-order
 * neighbours of one another. */
export interface RowSides {
  readonly prev?: string;
  readonly next?: string;
  readonly head?: string;
  readonly after?: string;
  readonly results?: boolean;
  /** The comment index that covers the whole list (the parent's), when the parent row passes it down. */
  readonly index?: CommentIndex | null;
}
export type AuthorNeighbors = () => RowSides;
/** How a list sits in the outline: `head`/`after`/`index` are passed down by the parent row that renders it. */
export interface ListRun { readonly head?: string; readonly after?: () => string | undefined; readonly results?: boolean; readonly index?: () => CommentIndex | null }

// Inline embeds are boundaries too. Read body AST nodes, so a macro example
// inside a code fence or hidden author metadata cannot split a run.
const embedBodies = new WeakMap<readonly Block[], boolean>();
function inlineEmbed(inlines: readonly Inline[]): boolean {
  return inlines.some(inline => {
    if (inline.k === "macro") return inline.name.toLowerCase() === "embed";
    if (inline.k === "emphasis" || inline.k === "subscript" || inline.k === "superscript" || inline.k === "tag") return inlineEmbed(inline.children);
    return inline.k === "link" && !!inline.label && inlineEmbed(inline.label);
  });
}
function itemEmbed(item: ListItem): boolean {
  return (!!item.name && inlineEmbed(item.name)) || item.content.some(blockEmbed) || item.items.some(itemEmbed);
}
function blockEmbed(block: Block): boolean {
  switch (block.kind) {
    case "paragraph": case "heading": case "bullet": case "footnote_def": return inlineEmbed(block.inline);
    case "quote": case "custom": return block.children.some(blockEmbed);
    case "list": return block.items.some(itemEmbed);
    case "table": return !!block.header?.some(inlineEmbed) || block.rows.some(row => row.some(inlineEmbed));
    default: return false;
  }
}
function hasEmbed(raw: string, format: "md" | "org"): boolean {
  if (!raw.includes("{{")) return false;
  const body = parseBody(raw, format);
  let found = embedBodies.get(body);
  if (found === undefined) { found = body.some(blockEmbed); embedBodies.set(body, found); }
  return found;
}

/** Map work is O(sibling IDs) only on membership/order changes. View filters may
 * emit equal new arrays: compare just those IDs, without re-reading authors.
 * Reused ID arrays short-circuit. No graph traversal, per-row indexOf, extra
 * context owner or retained page cache. */
export function authorSiblingNeighbors(ids: () => readonly string[], list: ListRun = {}): { ids: () => readonly string[]; neighbours: (id: string) => AuthorNeighbors } {
  const members = createMemo(ids, undefined, { equals: (before, after) => before === after
    || before.length === after.length && before.every((id, index) => id === after[index]) });
  const positions = createMemo(() => {
    const rows = members();
    return new Map(rows.map((id, i) => [id, [rows[i - 1], rows[i + 1]] as const]));
  });
  return { ids: members, neighbours: id => () => {
    const [prev, next] = positions().get(id) ?? [undefined, undefined];
    return { prev, next, head: prev === undefined ? list.head : undefined,
      after: next === undefined ? list.after?.() : undefined, results: list.results, index: list.index?.() };
  } };
}

// Two rows hold the same author only by one name: trimmed, spaces collapsed, case ignored (the key
// that also picks the tint). Quoted comments and rows with an inline embed never join a run.
function sameAuthor(left: string, right: string, format: "md" | "org"): boolean {
  const a = facetsOf(node(left)!.raw, format).properties, b = facetsOf(node(right)!.raw, format).properties;
  const by = authorOf(a), other = authorOf(b);
  if (by === null || other === null || authorKey(by) !== authorKey(other) || quoteSelectorOf(a) !== null || quoteSelectorOf(b) !== null) return false;
  return !hasEmbed(node(left)!.raw, format) && !hasEmbed(node(right)!.raw, format);
}

/** The sibling card (the tint's shared card): only adjacent rendered siblings connect. Parser facets are
 * cached; typing changes at most this row and the two neighbours, not a page-wide run. Children (including
 * threads) end the preceding card even when folded, so a card never claims authorship across a nesting or
 * comment boundary. */
export function authorRunContinues(leftId: string | undefined, rightId: string | undefined, format: "md" | "org"): boolean {
  if (!leftId || !rightId) return false;
  const left = node(leftId), right = node(rightId);
  if (!left || !right || left.parent !== right.parent || left.page !== right.page || left.children.length) return false;
  return sameAuthor(leftId, rightId, format);
}

/** Two rows that are adjacent in reading order (one directly above the other) and written by one author. */
export function authorThreadLinks(aboveId: string, belowId: string, format: "md" | "org"): boolean {
  const above = node(aboveId), below = node(belowId);
  return !!above && !!below && above.page === below.page && sameAuthor(aboveId, belowId, format);
}

/** The last row drawn inside `id`'s subtree: follow the last rendered child while the row shows its children.
 * Rendered children are the stored ones minus margin comments (membership from the covering comment index,
 * `isComment`); a sheet host draws no rows. Each row lies on the spine of at most one previous sibling, so a
 * page of n rows costs O(n) in all. The caller memoizes it per row: the answer changes only when a spine row's
 * children, a fold or comment membership changes, never when text is typed. */
export function lastRow(id: string, folded: (id: string) => boolean, isComment: (id: string) => boolean): string {
  for (let at = id;;) {
    const n = node(at);
    if (!n || folded(at) || blockIsOpaqueSheetView(at)) return at;
    let last: string | undefined;
    for (let i = n.children.length - 1; i >= 0 && last === undefined; i--) if (!isComment(n.children[i])) last = n.children[i];
    if (last === undefined) return at;
    at = last;
  }
}

/** Whether `row` joins the row above it and the row below it in a thread (reading order, any depth).
 * Above: `previousLast` (the previous sibling's last row, `lastRow`), else the parent. Below: the first
 * rendered child, else the next sibling, else the row below the parent's subtree. Hits of a results list never
 * join one another; inside a hit the outline rules apply (the list passes `results` to its own rows only). */
export function authorThreadSides(row: string, sides: RowSides, firstChild: string | undefined, previousLast: string | undefined, format: "md" | "org"): { continued: boolean; continues: boolean } {
  const above = sides.prev === undefined ? sides.head : previousLast;
  const below = firstChild ?? (sides.results ? undefined : sides.next ?? sides.after);
  return {
    continued: above !== undefined && authorThreadLinks(above, row, format),
    continues: below !== undefined && authorThreadLinks(row, below, format),
  };
}
