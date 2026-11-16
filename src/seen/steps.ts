// Previous / Next change in the seen notice (ADR 0079, D-20261029-SS): which
// marked items a page has in the order the page draws them, and which one a
// step lands on. Pure; the DOM half is reveal.ts.
import { SEEN_PAGE_ROOT, type SeenMarks } from "./marks";

/** One stop: a still-unedited changed block, or a removed-block marker. */
export interface SeenItem {
  /** Stable across re-reads of the same marks: `b:<block>` for a changed block,
   *  `a:<block>:<n>` / `f:<parent>:<n>` for the n-th marker drawn after a block
   *  or first inside a parent (empty parent = the page's own list). */
  key: string;
  kind: "block" | "removed";
  /** The block whose row holds the item (a marker lives in its anchor's row),
   *  or null for a marker drawn first in the page's own list. */
  reveal: string | null;
}

/** The items of one page in draw order: a block's own mark, the markers drawn
 *  first inside it, its children, then the markers drawn after it. A heavy edit
 *  (changed block plus the marker at its place) is the marker only, as the
 *  header counts it. O(blocks of the page); call it on a step, never per render. */
export function orderedChangeItems(
  marks: SeenMarks,
  roots: readonly string[],
  childrenOf: (id: string) => readonly string[],
  rawOf: (id: string) => string | undefined,
): SeenItem[] {
  const out: SeenItem[] = [];
  const markers = (bucket: SeenMarks["first"], id: string, prefix: "f" | "a") => {
    (bucket[id] ?? []).forEach((_, n) => out.push({ key: `${prefix}:${id}:${n}`, kind: "removed", reveal: id === SEEN_PAGE_ROOT ? null : id }));
  };
  const visit = (id: string) => {
    if (marks.blocks[id] !== undefined && rawOf(id) === marks.blocks[id] && !marks.paired[id]) {
      out.push({ key: `b:${id}`, kind: "block", reveal: id });
    }
    markers(marks.first, id, "f");
    for (const child of childrenOf(id)) visit(child);
    markers(marks.after, id, "a");
  };
  markers(marks.first, SEEN_PAGE_ROOT, "f");
  for (const root of roots) visit(root);
  return out;
}

/** Where the last step landed: the item's key, and its index when it did. */
export interface SeenCursor {
  key: string;
  index: number;
}

export interface SeenStep {
  item: SeenItem;
  index: number;
  /** The step crossed the end of the list and came round to the other end. */
  wrapped: boolean;
}

/** The item a step of `direction` (+1 next, -1 previous) lands on. With no
 *  cursor, next is the first item and previous the last. A cursor whose item
 *  is gone (edited away meanwhile) steps from the place it held. */
export function stepTarget(items: readonly SeenItem[], cursor: SeenCursor | null, direction: 1 | -1): SeenStep | null {
  const count = items.length;
  if (count === 0) return null;
  const pick = (index: number, wrapped: boolean): SeenStep => ({ item: items[index], index, wrapped });
  if (!cursor) return direction > 0 ? pick(0, false) : pick(count - 1, false);
  const at = items.findIndex((item) => item.key === cursor.key);
  const next = at >= 0 ? at + 1 : cursor.index; // a vanished item's successor took its place
  const previous = (at >= 0 ? at : Math.min(cursor.index, count)) - 1;
  if (direction > 0) return next >= count ? pick(0, true) : pick(next, false);
  return previous < 0 ? pick(count - 1, true) : pick(previous, false);
}
