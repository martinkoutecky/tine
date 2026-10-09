// The parent-side half of margin comments (vision §3.7): the source byte ranges
// of the passages its comment children quote, so the inline renderer can mark
// them. Derived at render time from the children's `quote::` and the parent's
// current raw text; nothing is stored.

import { createContext } from "solid-js";
import { anchorQuote, quoteSelectorOf } from "../comments";
import { node as docNode } from "../document";
import { facetsOf } from "./facets";
import { utf8ByteLength } from "./spans";
import type { Format } from "./ast";

/** Byte ranges [start, end) in the parent's rebulleted parse source. */
export interface QuoteHighlight {
  blockId: string;
  ranges: () => readonly (readonly [number, number])[];
}

export const QuoteHighlightContext = createContext<QuoteHighlight | null>(null);

/** Where the passages quoted by `parentId`'s comment children sit in the parse
 * source of its raw text (`"- "` + raw without leading whitespace, the form the
 * inline spans count in). A stale or whole-block quote contributes nothing.
 * Cost O(children) facet lookups plus O(raw × occurrences) per comment child. */
export function quotedSourceRanges(parentId: string, format: Format): [number, number][] {
  const parent = docNode(parentId);
  if (!parent) return [];
  const raw = parent.raw;
  const lead = raw.length - raw.trimStart().length;
  const out: [number, number][] = [];
  for (const childId of parent.children) {
    const child = docNode(childId);
    if (!child) continue;
    const selector = quoteSelectorOf(facetsOf(child.raw, format).properties);
    if (!selector) continue;
    const anchor = anchorQuote(raw, selector);
    if (anchor.kind !== "range" || anchor.start < lead) continue;
    const start = utf8ByteLength(raw.slice(lead, anchor.start)) + 2;
    out.push([start, start + utf8ByteLength(raw.slice(anchor.start, anchor.end))]);
  }
  return out;
}
