// What one block's raw text says about it as a comment: whether it is one, its
// selector and its resolution (vision §3.7). A pure function of the text; the
// one owner that keeps these answers current for a surface, and that every
// reader asks, is `commentIndex.ts` (GH #891 review round 3, PRINCIPLES 12).

import { quoteSelectorOf, resolvedOf, type QuoteSelector } from "./comments";
import { facetsOf } from "./render/facets";
import type { Format } from "./render/ast";

export interface CommentChild { readonly id: string; readonly selector: QuoteSelector; readonly resolved: boolean }
export interface CommentChildren {
  /** Comment roots among the children, in file order. */
  readonly entries: readonly CommentChild[];
  has(id: string): boolean;
}

export const NO_COMMENT_CHILDREN: CommentChildren = { entries: [], has: () => false };

// Screen only the key; the parser owns which separators/layout are accepted.
const MAY_QUOTE = /tine\.quote/i;

/** The comment entry for one child's raw text, or null for an ordinary block. */
export function commentChildOf(id: string, raw: string, format: Format): CommentChild | null {
  if (!MAY_QUOTE.test(raw)) return null;
  const properties = facetsOf(raw, format).properties;
  const selector = quoteSelectorOf(properties);
  return selector ? { id, selector, resolved: resolvedOf(properties) } : null;
}

export function sameChild(a: CommentChild | undefined, b: CommentChild | null): boolean {
  if (!a || !b) return !a && !b;
  return a.resolved === b.resolved && a.selector.quote === b.selector.quote
    && a.selector.prefix === b.selector.prefix && a.selector.suffix === b.selector.suffix;
}
