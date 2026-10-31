// The parent-side half of margin comments (vision §3.7): the source byte ranges
// of the passages its comment children quote, so the inline renderer can mark
// them. Derived at render time from the children's `quote::` and the parent's
// current raw text; nothing is stored.

import { createContext, createMemo, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import { anchorQuote, quoteSelectorOf, type QuoteSelector } from "../comments";
import { node as docNode, onBlockRawChanged } from "../document";
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
    const selector = quoteSelectorOf(facetsOf(child.raw, format).properties);
    if (!selector) continue;
    selectors.push([childId, selector]);
  }
  return rangesForSelectors(parent.raw, selectors);
}

function rangesForSelectors(raw: string, selectors: Iterable<readonly [string, QuoteSelector]>): QuotedRange[] {
  const lead = raw.length - raw.trimStart().length;
  const out: QuotedRange[] = [];
  for (const [childId, selector] of selectors) {
    const anchor = anchorQuote(raw, selector);
    if (anchor.kind !== "range" || anchor.start < lead) continue;
    const start = utf8ByteLength(raw.slice(lead, anchor.start)) + 2;
    out.push([start, start + utf8ByteLength(raw.slice(anchor.start, anchor.end)), childId]);
  }
  return out;
}

/** Follow only changes to actual quote selectors. Sheet rows are ordinary
 * children too: typing in one must not rescan every sibling for comments. */
export function createQuotedSourceRanges(parentId: Accessor<string | null>, format: Accessor<Format>): Accessor<QuotedRange[]> {
  const [revision, setRevision] = createSignal(0);
  const children = createMemo(() => {
    const id = parentId();
    const parent = id ? docNode(id) : null;
    const fmt = format();
    const positions = new Map<string, number>();
    const selectors = new Map<string, QuoteSelector>();
    for (const childId of parent?.children ?? []) {
      positions.set(childId, positions.size);
      const child = docNode(childId);
      const selector = child && untrack(() => quoteSelectorOf(facetsOf(child.raw, fmt).properties));
      if (selector) selectors.set(childId, selector);
    }
    return { positions, selectors, fmt };
  });
  onCleanup(onBlockRawChanged(id => {
    const current = children();
    if (!current.positions.has(id)) return;
    const child = docNode(id);
    const selector = child ? quoteSelectorOf(facetsOf(child.raw, current.fmt).properties) : null;
    const previous = current.selectors.get(id);
    if ((!selector && !previous) || selector && previous && selector.quote === previous.quote
      && selector.prefix === previous.prefix && selector.suffix === previous.suffix) return;
    if (selector) current.selectors.set(id, selector); else current.selectors.delete(id);
    setRevision(value => value + 1);
  }));
  return createMemo(() => {
    const current = children();
    revision();
    if (!current.selectors.size) return [];
    const id = parentId();
    const parent = id ? docNode(id) : null;
    return parent ? rangesForSelectors(parent.raw, [...current.selectors]
      .sort(([a], [b]) => current.positions.get(a)! - current.positions.get(b)!)) : [];
  });
}
