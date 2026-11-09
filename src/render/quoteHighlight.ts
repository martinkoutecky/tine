// The parent-side half of margin comments (vision §3.7): the source byte ranges
// of the passages its comment children quote, so the inline renderer can mark
// them. Derived at render time from the children's `tine.quote::` and the parent's
// current raw text; nothing is stored.

import { createContext, createMemo, type Accessor } from "solid-js";
import { anchorQuote, quoteSelectorOf, resolvedOf, type QuoteSelector } from "../comments";
import { node as docNode } from "../document";
import { createCommentChildren, type CommentChildren } from "../commentChildren";
import { facetsOf } from "./facets";
import { utf8ByteLength } from "./spans";
import type { Format } from "./ast";

/** A quoted passage: byte range [start, end) in the parent's rebulleted parse
 * source, and the comment that quotes it. */
export type QuotedRange = readonly [start: number, end: number, commentId: string];

export interface QuoteHighlight {
  blockId: string;
  ranges: () => readonly QuotedRange[];
}

export const QuoteHighlightContext = createContext<QuoteHighlight | null>(null);

function openSelector(raw: string, format: Format): QuoteSelector | null {
  const properties = facetsOf(raw, format).properties;
  return resolvedOf(properties) ? null : quoteSelectorOf(properties);
}

/** Where the passages quoted by `parentId`'s comment children sit in the parse
 * source of its raw text (`"- "` + raw without leading whitespace, the form the
 * inline spans count in). A stale or whole-block quote contributes nothing.
 * Cost O(children) facet lookups plus O(raw × occurrences) per comment child. */
export function quotedSourceRanges(parentId: string, format: Format): QuotedRange[] {
  const parent = docNode(parentId);
  if (!parent) return [];
  const selectors: [string, QuoteSelector][] = [];
  for (const childId of parent.children) {
    const child = docNode(childId);
    if (!child) continue;
    const selector = openSelector(child.raw, format);
    if (!selector) continue;
    selectors.push([childId, selector]);
  }
  return rangesForSelectors(parent.raw, selectors, format);
}

export function rangesForSelectors(raw: string, selectors: Iterable<readonly [string, QuoteSelector]>, format: Format): QuotedRange[] {
  const lead = raw.length - raw.trimStart().length;
  const out: QuotedRange[] = [];
  for (const [childId, selector] of selectors) {
    const anchor = anchorQuote(raw, selector, format);
    if (anchor.kind !== "range" || anchor.start < lead) continue;
    const start = utf8ByteLength(raw.slice(lead, anchor.start)) + 2;
    out.push([start, start + utf8ByteLength(raw.slice(anchor.start, anchor.end)), childId]);
  }
  return out;
}

/** Follow only changes to actual quote selectors (`createCommentChildren`).
 * Sheet rows are ordinary children too: typing in one must not rescan every
 * sibling for comments. Pass the block's own producer to share it. */
export function createQuotedSourceRanges(parentId: Accessor<string | null>, format: Accessor<Format>, source?: Accessor<CommentChildren>): Accessor<QuotedRange[]> {
  const comments = source ?? createCommentChildren(parentId, format);
  return createMemo(() => {
    const open = comments().entries.filter(entry => !entry.resolved);
    if (!open.length) return [];
    const id = parentId();
    const parent = id ? docNode(id) : null;
    return parent ? rangesForSelectors(parent.raw, open.map(entry => [entry.id, entry.selector] as const), format()) : [];
  });
}
