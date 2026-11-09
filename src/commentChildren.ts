// Which children of a block are comment thread roots, with their selectors and
// resolution (vision §3.7; GH #891 review finding 1, PC-1/PC-4/PC-5). One
// producer feeds the body outline's filter, the quote highlights, the compact
// entry point and the margin column, and it follows deltas the way
// `createQuotedSourceRanges` did: a structural change (children added, removed,
// moved) rescans the children once with untracked reads; a raw edit of one child
// re-derives only that child and notifies only when its membership, selector or
// resolution changed. Typing in an ordinary sibling therefore costs one regex
// test, whatever the number of siblings, and subscribes nothing to the others.

import { createMemo, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import { quoteSelectorOf, resolvedOf, type QuoteSelector } from "./comments";
import { node as docNode, onBlockRawChanged } from "./document";
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

function sameChild(a: CommentChild | undefined, b: CommentChild | null): boolean {
  if (!a || !b) return !a && !b;
  return a.resolved === b.resolved && a.selector.quote === b.selector.quote
    && a.selector.prefix === b.selector.prefix && a.selector.suffix === b.selector.suffix;
}

/** O(children) untracked work per structural change of `parentId`; O(1) per raw
 * edit of an ordinary child; one facet lookup per raw edit of a comment child. */
export function createCommentChildren(parentId: Accessor<string | null>, format: Accessor<Format>): Accessor<CommentChildren> {
  const [revision, setRevision] = createSignal(0);
  const structure = createMemo(() => {
    const id = parentId();
    const parent = id ? docNode(id) : null;
    const children = parent?.children ?? [];
    const fmt = format();
    const found = new Map<string, CommentChild>();
    for (const childId of children) {
      // Track node replacement (page reloads), never a child's text.
      const child = docNode(childId);
      const entry = child && untrack(() => commentChildOf(childId, child.raw, fmt));
      if (entry) found.set(childId, entry);
    }
    if (children.length) onCleanup(onBlockRawChanged(changed => {
      const child = docNode(changed);
      const next = child ? commentChildOf(changed, child.raw, fmt) : null;
      if (sameChild(found.get(changed), next)) return;
      if (next) found.set(changed, next); else found.delete(changed);
      setRevision(value => value + 1);
    }, children));
    return { children, found };
  });
  return createMemo<CommentChildren>(previous => {
    const { children, found } = structure();
    revision();
    if (!found.size) return NO_COMMENT_CHILDREN;
    const entries = children.flatMap(id => found.get(id) ?? []);
    if (previous.entries.length === entries.length && previous.entries.every((entry, i) => entry === entries[i])) return previous;
    const ids = new Set(entries.map(entry => entry.id));
    return { entries, has: id => ids.has(id) };
  }, NO_COMMENT_CHILDREN);
}
