import { createMemo } from "solid-js";
import { node } from "../document";
import { authorOf, quoteSelectorOf } from "../comments";
import { facetsOf, parseBody } from "../render/facets";
import type { Block, Inline, ListItem } from "../render/ast";

export type AuthorNeighbors = () => readonly [string | undefined, string | undefined];

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
export function authorSiblingNeighbors(ids: () => readonly string[]): { ids: () => readonly string[]; neighbours: (id: string) => AuthorNeighbors } {
  const members = createMemo(ids, undefined, { equals: (before, after) => before === after
    || before.length === after.length && before.every((id, index) => id === after[index]) });
  const positions = createMemo(() => {
    const list = members();
    return new Map(list.map((id, i) => [id, [list[i - 1], list[i + 1]] as const]));
  });
  return { ids: members, neighbours: id => () => positions().get(id) ?? [undefined, undefined] };
}

/** Only the adjacent rendered siblings may connect. Parser facets are cached;
 * typing changes at most this row and the two neighbours, not a page-wide run.
 * Children (including threads) end the preceding line even when folded, so a
 * line never claims authorship across a nesting/comment boundary. */
export function authorRunContinues(leftId: string | undefined, rightId: string | undefined, format: "md" | "org"): boolean {
  if (!leftId || !rightId) return false;
  const left = node(leftId), right = node(rightId);
  if (!left || !right || left.parent !== right.parent || left.page !== right.page || left.children.length) return false;
  const a = facetsOf(left.raw, format).properties, b = facetsOf(right.raw, format).properties;
  const by = authorOf(a);
  if (by === null || by !== authorOf(b) || quoteSelectorOf(a) !== null || quoteSelectorOf(b) !== null) return false;
  return !hasEmbed(left.raw, format) && !hasEmbed(right.raw, format);
}
