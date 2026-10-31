// Inline presentation of margin comments (vision §3.7, slice 1): the quote
// header of a comment card, the author chip, the thread context that tints a
// comment's replies, and the provider that marks quoted passages in the parent.
// All of it is derived at render time from ordinary block properties. Slice 2
// adds the main column's marker for comments drawn in the margin.

import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, useContext, type JSX } from "solid-js";
import { MarginContext } from "./marginContext";
import { DialogActions } from "./DialogActions";
import { t } from "../i18n";
import { anchorQuote, quoteSelectorOf, type QuoteSelector } from "../comments";
import { facetsOf } from "../render/facets";
import { AUTHOR_TINTS, authorTint } from "../commentIdentity";
import { blockPageReadOnly, blockWritable, node as docNode } from "../document";
import { commentFromReplyDraft, deleteComment, submitReply } from "../commentActions";
import { internalLinkMouseDown } from "../linkGesture";
import { shouldOpenBlockContextMenu } from "../contextMenuPolicy";
import { openContextMenu } from "../ui";
import { beginDrag } from "./blockGestures";
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
export function CommentQuoteHeader(props: { parentId: string | null; selector: QuoteSelector | null; controls?: JSX.Element }): JSX.Element {
  return <Show when={props.selector !== null && props.parentId !== null}><QuoteHeader parentId={props.parentId!} selector={props.selector!} controls={props.controls} /></Show>;
}

function QuoteHeader(props: { parentId: string; selector: QuoteSelector; controls?: JSX.Element }): JSX.Element {
  const anchor = createMemo(() => {
    const parent = docNode(props.parentId);
    return parent ? anchorQuote(parent.raw, props.selector) : { kind: "stale" as const };
  });
  return (
    <div class="comment-quote" classList={{ stale: anchor().kind === "stale" }}>
      <Show when={anchor().kind !== "whole"} fallback={<span class="comment-quote-text whole">{t("comment-on-whole-block")}</span>}>
        <span class="comment-quote-text" title={props.selector.quote}>
          <Show when={anchor().kind === "stale"} fallback={props.selector.quote}>
            <s>{props.selector.quote}</s>
          </Show>
        </span>
        <Show when={anchor().kind === "stale"}>
          <span class="comment-quote-stale">{t("comment-quote-changed")}</span>
        </Show>
      </Show>
      {props.controls}
    </div>
  );
}

/** Who wrote a block: the `author::` value, or "you" for the graph owner. On a
 * comment block (`handleOf` set) the chip also takes over what the bullet did:
 * it starts the block drag (the same gesture contract as the bullet), and its
 * long press opens the block menu (contextMenuPolicy). Right-click anywhere on
 * the row already opens the menu, where "Zoom into block" lives. */
export function AuthorChip(props: { author: string | null; handleOf?: string }): JSX.Element {
  const onMouseDown = (e: MouseEvent) => {
    const id = props.handleOf;
    if (id === undefined) return;
    // Not an edit gesture: keep the row's caret placement out of it.
    e.stopPropagation();
    internalLinkMouseDown(e);
    if (e.button !== 0) return;
    if (!blockPageReadOnly(id)) beginDrag(id, e);
  };
  return (
    <span
      class="author-chip"
      classList={{ agent: props.author !== null, "comment-handle": props.handleOf !== undefined, ...tintClasses(props.author) }}
      title={props.author !== null ? t("comment-written-by", { author: props.author }) : t("comment-written-by-you")}
      onMouseDown={onMouseDown}
      onContextMenu={e => {
        if (props.handleOf === undefined || !shouldOpenBlockContextMenu(e.target)) return;
        e.preventDefault();
        e.stopPropagation();
        openContextMenu(e.clientX, e.clientY, props.handleOf);
      }}
    >
      {props.author ?? t("comment-author-you")}
    </span>
  );
}

function tintClasses(author: string | null): Record<string, boolean> {
  const tint = author === null ? -1 : authorTint(author);
  const classes: Record<string, boolean> = {};
  for (let i = 0; i < AUTHOR_TINTS; i++) classes[`author-tint-${i}`] = i === tint;
  return classes;
}

/** Delete shares the first line but consumes its gesture without placing a caret. */
export function CommentActions(props: { id: string }): JSX.Element {
  return <Show when={blockWritable(props.id)}>
    <div class="comment-actions" onMouseDown={(e) => e.stopPropagation()}>
      <button type="button" class="comment-delete" title={t("comment-delete")} aria-label={t("comment-delete")} onClick={(e) => {
        e.stopPropagation();
        deleteComment(props.id);
      }}>×</button>
    </div>
  </Show>;
}

/** No document node exists while composing: abandoning an empty draft costs
 * zero writes. Once submitted it is an ordinary editable child block. */
export function CommentComposer(props: { id: string; surface: string }): JSX.Element {
  const [draft, setDraft] = createSignal("");
  const [expanded, setExpanded] = createSignal(false);
  let input!: HTMLTextAreaElement;
  const cancel = () => { setDraft(""); setExpanded(false); input.blur(); };
  const submit = () => { if (submitReply(props.id, draft())) cancel(); };
  return <Show when={blockWritable(props.id)}>
    <div class="comment-composer" classList={{ expanded: expanded() }} onMouseDown={e => e.stopPropagation()} onClick={e => e.stopPropagation()}>
      <textarea ref={input} class="comment-reply-draft" rows={expanded() ? 3 : 1}
        placeholder={t("comment-reply-placeholder")} aria-label={t("comment-reply-placeholder")}
        value={draft()} onInput={e => setDraft(e.currentTarget.value)}
        onFocus={() => setExpanded(true)}
        onBlur={e => { if (!draft().trim() && !e.currentTarget.parentElement?.contains(e.relatedTarget as Node | null)) setExpanded(false); }}
        onKeyDown={e => {
          e.stopPropagation();
          if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "r") {
            e.preventDefault();
            if (commentFromReplyDraft(props.id, draft(), input.selectionStart, input.selectionEnd, props.surface)) cancel();
          }
          else if (e.key === "Escape") { e.preventDefault(); cancel(); }
          else if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); submit(); }
        }} />
      <Show when={expanded()}><DialogActions class="comment-composer-footer"
        cancel={<button type="button" onClick={cancel}>{t("confirm-cancel")}</button>}
        primary={<button type="button" class="comment-reply" disabled={!draft().trim()} onClick={submit}>{t("comment-reply")}</button>}
      /></Show>
    </div>
  </Show>;
}

function sameRanges(a: readonly QuotedRange[], b: readonly QuotedRange[]): boolean {
  return a.length === b.length && a.every(([start, end, id], i) => start === b[i][0] && end === b[i][1] && id === b[i][2]);
}

/** Mark the passages `parentId`'s comment children quote inside the rendering
 * `children()` draws; a block with no children (`when` false) has none to mark
 * and draws it plainly. */
export function QuoteHighlightProvider(props: { parentId: string; format: Format; when: boolean; children: () => JSX.Element }): JSX.Element {
  // Equal ranges keep the old array, so typing in a child does not re-render
  // the parent's text runs.
  const ranges = createMemo(() => quotedSourceRanges(props.parentId, props.format), [], { equals: sameRanges });
  return (
    <Show when={props.when} fallback={props.children()}>
      <QuoteHighlightContext.Provider value={{ blockId: props.parentId, ranges }}>
        {props.children()}
      </QuoteHighlightContext.Provider>
    </Show>
  );
}

/** At the right edge of a block whose comments are drawn in the margin: how
 * many there are. While mounted it asks the margin to draw them, and it asks
 * for a re-measure when the block's text or a comment changes (the quoted
 * passage can move without the page resizing). Clicking emphasises the first
 * thread and its passage. */
export function MarginMarker(props: { parentId: string; comments: readonly string[] }): JSX.Element {
  return <Show when={props.comments.length > 0}><MarkerButton parentId={props.parentId} comments={props.comments} /></Show>;
}

function MarkerButton(props: { parentId: string; comments: readonly string[] }): JSX.Element {
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
  const label = () => t("comment-margin-count", { count: props.comments.length });
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
