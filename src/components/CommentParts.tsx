// Inline presentation of margin comments (vision §3.7, slice 1): the quote
// header of a comment card, the author chip, the thread context that tints a
// comment's replies, and the provider that marks quoted passages in the parent.
// All of it is derived at render time from ordinary block properties.

import { createMemo, Show, type JSX } from "solid-js";
import { anchorQuote, quoteSelectorOf, type QuoteSelector } from "../comments";
import { facetsOf } from "../render/facets";
import { node as docNode } from "../document";
import { QuoteHighlightContext, quotedSourceRanges } from "../render/quoteHighlight";
import type { Format } from "../render/ast";

/** Whether the block under `parentId` belongs to a comment's thread: some
 * ancestor (from `parentId` up) is a comment. O(depth) facet lookups. */
export function inCommentThread(parentId: string | null, format: Format): boolean {
  for (let id = parentId; id !== null;) {
    const n = docNode(id);
    if (!n) return false;
    if (n.parent !== null && quoteSelectorOf(facetsOf(n.raw, format).properties) !== null) return true;
    id = n.parent;
  }
  return false;
}

/** The muted quote line above a comment: the quoted passage, "whole block", or
 * the quote struck through when it no longer occurs in the parent. */
export function CommentQuoteHeader(props: { parentId: string; selector: QuoteSelector }): JSX.Element {
  const anchor = createMemo(() => {
    const parent = docNode(props.parentId);
    return parent ? anchorQuote(parent.raw, props.selector) : { kind: "stale" as const };
  });
  return (
    <div class="comment-quote" classList={{ stale: anchor().kind === "stale" }}>
      <Show when={anchor().kind !== "whole"} fallback={<span class="comment-quote-text whole">on the whole block</span>}>
        <span class="comment-quote-text" title={props.selector.quote}>
          <Show when={anchor().kind === "stale"} fallback={props.selector.quote}>
            <s>{props.selector.quote}</s>
          </Show>
        </span>
        <Show when={anchor().kind === "stale"}>
          <span class="comment-quote-stale">quoted text changed</span>
        </Show>
      </Show>
    </div>
  );
}

/** Who wrote a block: the `author::` value, or "you" for the graph owner. */
export function AuthorChip(props: { author: string | null }): JSX.Element {
  return (
    <span
      class="author-chip"
      classList={{ agent: props.author !== null }}
      title={props.author !== null ? `Written by ${props.author}` : "Written by you"}
    >
      {props.author ?? "you"}
    </span>
  );
}

function sameRanges(a: readonly (readonly [number, number])[], b: readonly (readonly [number, number])[]): boolean {
  return a.length === b.length && a.every(([start, end], i) => start === b[i][0] && end === b[i][1]);
}

/** Mark the passages `parentId`'s comment children quote inside its rendering. */
export function QuoteHighlightProvider(props: { parentId: string; format: Format; children: JSX.Element }): JSX.Element {
  // Equal ranges keep the old array, so typing in a child does not re-render
  // the parent's text runs.
  const ranges = createMemo(() => quotedSourceRanges(props.parentId, props.format), [], { equals: sameRanges });
  return (
    <QuoteHighlightContext.Provider value={{ blockId: props.parentId, ranges }}>
      {props.children}
    </QuoteHighlightContext.Provider>
  );
}
