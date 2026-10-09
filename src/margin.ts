// Margin dialogue, slice 2 (vision 2026-10 §3.7 "Layout"): on a wide main pane,
// a page holding comments gets a right-hand comment column. This module is the
// model side: which blocks belong in the margin, how threads stack, and where the
// arrow keys go when part of the outline is drawn beside it. Everything is
// derived from the outline at render time; nothing is persisted.

import { quoteSelectorOf } from "./comments";
import { facetsOf } from "./render/facets";
import { nextVisible, prevVisible, node as docNode, type OutlineScope } from "./document";
import type { Format } from "./render/ast";

/** The narrowest main pane (its scroller's width, px) that gets the margin. The
 * main column keeps at least ~550 px beside a 280 px column, a 32 px gap and the
 * pane's 48 px side padding. Mirrored by `--margin-column-*` in comments.css. */
export const MARGIN_MIN_PANE_WIDTH = 960;
/** Vertical space between two stacked threads, px. */
export const MARGIN_THREAD_GAP = 8;

/** Whether `id` is a comment: it has a parent block and a `quote::`. */
export function isCommentId(id: string, format: Format): boolean {
  const n = docNode(id);
  return !!n && n.parent !== null && quoteSelectorOf(facetsOf(n.raw, format).properties) !== null;
}

/** Whether some block of the page is a comment. Stops at the first one, so on a
 * page with comments it tracks only the blocks before it; a page without any
 * pays one cached facet lookup per block when a block changes. */
export function pageHasComment(roots: readonly string[], format: Format): boolean {
  const visit = (ids: readonly string[]): boolean => {
    for (const id of ids) {
      if (isCommentId(id, format)) return true;
      const n = docNode(id);
      if (n && visit(n.children)) return true;
    }
    return false;
  };
  return visit(roots);
}

/** The comment whose thread holds `id` (itself or its nearest comment
 * ancestor), or null for an ordinary block. O(depth). */
export function threadRootOf(id: string, format: Format): string | null {
  for (let at: string | null = id; at !== null; at = docNode(at)?.parent ?? null) {
    if (isCommentId(at, format)) return at;
  }
  return null;
}

/** Top offsets for threads already sorted in document order: each sits at its
 * anchor unless the previous one is still in the way, then directly below it.
 * Nothing overlaps. O(threads). */
export function stackThreads(items: readonly { anchor: number; height: number }[], gap = MARGIN_THREAD_GAP): number[] {
  const tops: number[] = [];
  let floor = -Infinity;
  for (const { anchor, height } of items) {
    const top = Math.max(anchor, floor);
    tops.push(top);
    floor = top + height + gap;
  }
  return tops;
}

/** Where a block's arrow-key exit goes while comments are drawn in the margin.
 * `thread` names the thread the block is drawn in, or null for the main column. */
export interface MarginNav {
  thread: { root: string; parent: string } | null;
  format: Format;
}

function mainStep(from: string, step: (id: string) => string | null, format: Format): string | null {
  let at = step(from);
  while (at !== null && threadRootOf(at, format) !== null) at = step(at);
  return at;
}

/** The block above `id` in the view: the main column skips threads; the top of a
 * thread returns to the commented block. */
export function marginPrev(id: string, scope: OutlineScope | null, nav: MarginNav): string | null {
  const step = (at: string) => prevVisible(at, scope);
  if (!nav.thread) return mainStep(id, step, nav.format);
  return id === nav.thread.root ? nav.thread.parent : step(id);
}

/** The block below `id` in the view: the main column skips threads; past the end
 * of a thread comes the main-column block after the commented one. */
export function marginNext(id: string, scope: OutlineScope | null, nav: MarginNav): string | null {
  const step = (at: string) => nextVisible(at, scope);
  if (!nav.thread) return mainStep(id, step, nav.format);
  const next = step(id);
  if (next !== null && insideThread(next, nav.thread.root)) return next;
  return mainStep(nav.thread.parent, step, nav.format);
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
