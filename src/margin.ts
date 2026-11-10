// Margin dialogue, slice 2 (vision 2026-10 §3.7 "Layout"): on a wide main pane,
// a page holding comments gets a right-hand comment column. This module is the
// model side: which blocks belong in the margin, how threads stack, and where the
// arrow keys go when part of the outline is drawn beside it. Everything is
// derived from the outline at render time, with a device-local display policy.

import { anchorQuote, type QuoteSelector } from "./comments";
import { commentChildOf } from "./commentChildren";
import { nextVisible, prevVisible, node as docNode, type OutlineScope } from "./document";
import type { Format } from "./render/ast";
import { bodyNeighbor } from "./document";
import { commentsInPopup } from "./commentMarginPref";
export { isCommentId, threadRootOf } from "./document";

/** The narrowest main pane (its scroller's width, px) that gets the margin. The
 * main column keeps at least ~550 px beside a 280 px column, a 32 px gap and the
 * pane's 48 px side padding. Mirrored by `--margin-column-*` in comments.css. */
export const MARGIN_MIN_PANE_WIDTH = 960;
/** Below this pane width there is no useful side rail: use a bottom sheet.
 * Measured in the same CSS pixels as margin fit, including interface zoom. */
export const COMMENT_PHONE_MAX_WIDTH = 480;
/** O(1) viewer policy: On click uses medium entry at wide widths too. */
export function commentTier(width: number): "wide" | "medium" | "phone" {
  return width <= COMMENT_PHONE_MAX_WIDTH ? "phone" : width >= MARGIN_MIN_PANE_WIDTH && !commentsInPopup() ? "wide" : "medium";
}
/** The comment column's width bounds, px (`--margin-column-width` in comments.css
 * is the narrow end). */
export const MARGIN_COLUMN_MIN = 280;
export const MARGIN_COLUMN_MAX = 440;
const MARGIN_COLUMN_GAP = 32;
// Kept free of the column at the pane's edge, beyond the content's own padding
// (a vertical scrollbar, rounding).
const PANE_SLACK = 16;

/** The column's width in a pane `paneWidth` px wide whose reading column is at
 * most `content` px, padding included (`--tine-main-content-max-width`; null: wide mode or an unknown cap, where the outline takes
 * whatever the column leaves). Spare width beyond outline, gap and padding goes to
 * the column, up to the maximum. Pure. */
export function marginColumnWidth(paneWidth: number, content: number | null): number {
  if (content === null) return MARGIN_COLUMN_MIN;
  const spare = paneWidth - content - MARGIN_COLUMN_GAP - PANE_SLACK;
  return Math.round(Math.min(MARGIN_COLUMN_MAX, Math.max(MARGIN_COLUMN_MIN, spare)));
}
/** Vertical space between two stacked threads, px. */
export const MARGIN_THREAD_GAP = 8;

/** Stable passage order, independent of creation/file order. Unanchored threads
 * stay reachable at the block's start; replies never contribute a thread.
 * `entries` are comment roots in file order (`createCommentChildren`). */
export function orderCommentEntries(parentRaw: string, entries: readonly { id: string; selector: QuoteSelector }[], format: Format): string[] {
  return entries.map(({ id, selector }) => {
    const anchor = anchorQuote(parentRaw, selector, format);
    return { id, at: anchor.kind === "range" ? anchor.start : -1 };
  }).sort((a, b) => a.at - b.at).map(item => item.id);
}

/** Imperative form for actions: scans the children once. */
export function orderedCommentIds(parentId: string, format: Format): string[] {
  const parent = docNode(parentId);
  if (!parent) return [];
  return orderCommentEntries(parent.raw, parent.children.flatMap(id => {
    const child = docNode(id);
    return (child && commentChildOf(id, child.raw, format)) || [];
  }), format);
}

/** Top offsets for threads already sorted in document order: each sits at its
 * anchor unless the previous one is still in the way, then directly below it.
 * Nothing overlaps. O(threads). */
export function stackThreads(items: readonly { anchor: number; height: number }[], gap = MARGIN_THREAD_GAP, focused = -1): number[] {
  const tops: number[] = [];
  let floor = -Infinity;
  for (const { anchor, height } of items) {
    const top = Math.max(anchor, floor);
    tops.push(top);
    floor = top + height + gap;
  }
  if (focused >= 0 && focused < items.length) {
    tops[focused] = items[focused].anchor;
    for (let i = focused - 1; i >= 0; i--)
      tops[i] = Math.min(items[i].anchor, tops[i + 1] - items[i].height - gap);
    for (let i = focused + 1; i < items.length; i++)
      tops[i] = Math.max(items[i].anchor, tops[i - 1] + items[i - 1].height + gap);
  }
  return tops;
}

/** Where a block's arrow-key exit goes while comments are drawn in the margin.
 * `thread` names the thread the block is drawn in, or null for the main column;
 * its `order` is the presented root order of that surface (review finding 2). */
export interface MarginNav {
  thread: { root: string; parent: string; order?: () => readonly string[] } | null;
  format: Format;
}

/** The block above `id` in the view: the main column skips threads; the top of a
 * thread goes to the end of the thread drawn above it, else to the commented block. */
export function marginPrev(id: string, scope: OutlineScope | null, nav: MarginNav): string | null {
  const step = (at: string) => prevVisible(at, scope);
  if (!nav.thread) return bodyNeighbor(id, -1, scope);
  if (id !== nav.thread.root) return step(id);
  const order = nav.thread.order?.() ?? [];
  const above = order[order.indexOf(id) - 1];
  return above !== undefined && order.indexOf(id) > 0 && docNode(above) ? lastInThread(above, scope) : nav.thread.parent;
}

/** The block below `id` in the view: the main column skips threads; past the end
 * of a thread comes the next thread drawn on the same surface, then the
 * main-column block after the commented one. */
export function marginNext(id: string, scope: OutlineScope | null, nav: MarginNav): string | null {
  const step = (at: string) => nextVisible(at, scope);
  if (!nav.thread) return bodyNeighbor(id, 1, scope);
  const next = step(id);
  if (next !== null && insideThread(next, nav.thread.root)) return next;
  const order = nav.thread.order?.() ?? [];
  const at = order.indexOf(nav.thread.root);
  const below = at >= 0 ? order[at + 1] : undefined;
  return below !== undefined && docNode(below) ? below : bodyNeighbor(nav.thread.parent, 1, scope);
}

/** The last visible message of a thread. O(thread size). */
function lastInThread(root: string, scope: OutlineScope | null): string {
  let last = root;
  for (let at = nextVisible(root, scope); at !== null && insideThread(at, root); at = nextVisible(at, scope)) last = at;
  return last;
}

function insideThread(id: string, root: string): boolean {
  for (let at: string | null = id; at !== null; at = docNode(at)?.parent ?? null) if (at === root) return true;
  return false;
}

// Instrumentation for the cost gate: how many margin measurement passes ran.
let measurePasses = 0;
export function countMarginMeasurePass(): void {
  measurePasses++;
}
export function marginMeasurePasses(): number {
  return measurePasses;
}
