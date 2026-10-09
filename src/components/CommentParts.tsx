// Inline presentation of margin comments (vision §3.7, slice 1): the quote
// header of a comment card, the author chip, the thread context that tints a
// comment's replies, and the provider that marks quoted passages in the parent.
// All of it is derived at render time from ordinary block properties. Slice 2
// adds the main column's marker for comments drawn in the margin.

import { createEffect, createMemo, onCleanup, onMount, Show, useContext, type JSX } from "solid-js";
import { MarginContext } from "./marginContext";
import { anchorQuote, quoteSelectorOf, type QuoteSelector } from "../comments";
import { facetsOf } from "../render/facets";
import { node as docNode } from "../document";
import { QuoteHighlightContext, quotedSourceRanges, type QuotedRange } from "../render/quoteHighlight";
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

function sameRanges(a: readonly QuotedRange[], b: readonly QuotedRange[]): boolean {
  return a.length === b.length && a.every(([start, end, id], i) => start === b[i][0] && end === b[i][1] && id === b[i][2]);
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

/** At the right edge of a block whose comments are drawn in the margin: how
 * many there are. While mounted it asks the margin to draw them, and it asks
 * for a re-measure when the block's text or a comment changes (the quoted
 * passage can move without the page resizing). Clicking emphasises the first
 * thread and its passage. */
export function MarginMarker(props: { parentId: string; comments: readonly string[] }): JSX.Element {
  const placement = useContext(MarginContext)!;
  let el!: HTMLButtonElement;
  onMount(() => {
    const row = el.closest<HTMLElement>(".block-main");
    if (!row) return;
    onCleanup(placement.api.register(props.parentId, row));
    // The block's text can render its quoted passages a frame after the row
    // (moving them inside a row of unchanged size): any change to the text's
    // DOM asks for a re-measure. Class toggles (emphasis) are not watched.
    const content = row.querySelector(".block-content-wrapper") ?? row;
    const Observer = (row.ownerDocument.defaultView as (Window & typeof globalThis) | null)?.MutationObserver;
    if (!Observer) return;
    const watcher = new Observer(() => placement.api.schedule());
    watcher.observe(content, { childList: true, subtree: true, characterData: true });
    onCleanup(() => watcher.disconnect());
  });
  createEffect(() => {
    void docNode(props.parentId)?.raw;
    for (const id of props.comments) void docNode(id)?.raw;
    placement.api.schedule();
  });
  const label = () => `${props.comments.length} comment${props.comments.length === 1 ? "" : "s"} in the margin`;
  return (
    <button
      type="button"
      class="margin-marker"
      classList={{ active: props.comments.includes(placement.api.activeComment() ?? "") }}
      ref={el}
      title={label()}
      aria-label={label()}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation();
        placement.api.setActiveComment(props.comments[0] ?? null);
      }}
    >
      {props.comments.length}
    </button>
  );
}
