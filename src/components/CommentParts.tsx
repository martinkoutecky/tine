// Inline presentation of margin comments (vision §3.7, slice 1): the quote
// header of a comment card, the author chip, the thread context that tints a
// comment's replies, and the provider that marks quoted passages in the parent.
// All of it is derived at render time from ordinary block properties. Slice 2
// adds the main column's marker for comments drawn in the margin.

import { createEffect, createMemo, createSignal, createUniqueId, onCleanup, onMount, Show, type Accessor, type JSX } from "solid-js";
import { DialogActions } from "./DialogActions";
import { t, activeLocale } from "../i18n";
import { anchorQuote, commentDateOf, type QuoteSelector } from "../comments";
import { AUTHOR_TINTS, authorTint } from "../commentIdentity";
import { blockWritable, formatForBlock, node as docNode } from "../document";
import { threadRootOf } from "../document";
import { registerReplyDraft } from "../commentMargin";
import { commentFromReplyDraft, deleteComment, submitReply, setThreadResolved } from "../commentActions";
import { runWithCommentName } from "../commentNamePrompt";
import { internalLinkMouseDown } from "../linkGesture";
import { shouldOpenBlockContextMenu } from "../contextMenuPolicy";
import { commentMarginHidden } from "../commentMarginPref";
import { openContextMenu } from "../ui";
import { QuoteHighlightContext, createQuotedSourceRanges, type QuoteCoverage, type QuotedRange } from "../render/quoteHighlight";
import type { Format } from "../render/ast";
import type { CommentChildren } from "../commentChildren";
import { consumeReplyFocus, replyFocusRequest } from "../commentReplyFocus";
import { commentDraftState } from "../commentDraftState";
import { threadResolved } from "../commentResolution";
import { facetsOf } from "../render/facets";
import { registerTransientLayer } from "../transientLayers";

/** Whether the block under `parentId` belongs to a comment's thread: some
 * ancestor (from `parentId` up) is a comment. O(depth) facet lookups. */
export function inCommentThread(parentId: string | null, format: Format): boolean {
  return parentId !== null && threadRootOf(parentId, format) !== null;
}

/** The muted quote line above a comment: the quoted passage, "whole block", or
 * the quote struck through when it no longer occurs in the parent. */
export function CommentQuoteHeader(props: { parentId: string | null; selector: QuoteSelector | null }): JSX.Element {
  return <Show when={props.selector !== null && props.parentId !== null}><QuoteHeader parentId={props.parentId!} selector={props.selector!} /></Show>;
}

function QuoteHeader(props: { parentId: string; selector: QuoteSelector }): JSX.Element {
  const anchor = createMemo(() => {
    const parent = docNode(props.parentId);
    // In-thread quotes are compact historical excerpts of another response.
    // New responses attach to the root, so their source need not be their parent.
    if (inCommentThread(props.parentId, formatForBlock(props.parentId))) return { kind: "compact" as const };
    return parent ? anchorQuote(parent.raw, props.selector, formatForBlock(props.parentId)) : { kind: "stale" as const };
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
        <Show when={anchor().kind === "ambiguous"}>
          <span class="comment-quote-stale">{t("comment-quote-ambiguous")}</span>
        </Show>
      </Show>
    </div>
  );
}

/** Every response starts with the same author handle and top-right actions;
 * quotes are context above this row and the body always starts below it. */
export function CommentHeader(props: { id: string; author: string | null }): JSX.Element {
  const date = () => commentDateOf(facetsOf(docNode(props.id)?.raw ?? "", formatForBlock(props.id)).properties);
  return <div class="comment-header">
    <AuthorChip author={props.author} handleOf={props.id} />
    <Show when={date()}>{date => <time class="comment-date" dateTime={date()} title={new Intl.DateTimeFormat(activeLocale(), { dateStyle: "long", timeStyle: "short" }).format(new Date(date()))}>
      {new Intl.DateTimeFormat(activeLocale(), { dateStyle: "short", timeStyle: "short" }).format(new Date(date()))}
    </time>}</Show>
    <CommentActions id={props.id} />
  </div>;
}

/** Who wrote a block: the `tine.author::` value, or "you" for the graph owner. On a
 * comment block (`handleOf` set) its long press opens the block menu
 * (contextMenuPolicy). Right-click anywhere on
 * the row already opens the menu, where "Zoom into block" lives. */
export function AuthorChip(props: { author: string | null; handleOf?: string; menuOf?: string }): JSX.Element {
  const onMouseDown = (e: MouseEvent) => {
    const id = props.handleOf;
    if (id === undefined) { if (props.menuOf) { e.stopPropagation(); if (e.button === 0) e.preventDefault(); } return; }
    // Not an edit gesture: keep the row's caret placement out of it.
    e.stopPropagation();
    internalLinkMouseDown(e);
  };
  return (
    <span
      class="author-chip"
      classList={{ agent: props.author !== null, "comment-handle": props.handleOf !== undefined, ...tintClasses(props.author) }}
      title={props.author !== null ? t("comment-written-by", { author: props.author }) : t("comment-written-by-you")}
      onMouseDown={onMouseDown}
      role={props.menuOf ? "button" : undefined}
      tabIndex={props.menuOf ? 0 : undefined}
      onKeyDown={e => { if (props.menuOf && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); e.stopPropagation(); const r = e.currentTarget.getBoundingClientRect(); openContextMenu(r.left, r.bottom, props.menuOf); } }}
      onClick={e => { if (props.menuOf) { e.stopPropagation(); openContextMenu(e.clientX, e.clientY, props.menuOf); } }}
      onContextMenu={e => {
        const id = props.menuOf ?? props.handleOf;
        if (id === undefined || (!props.menuOf && !shouldOpenBlockContextMenu(e.target))) return;
        e.preventDefault();
        e.stopPropagation();
        openContextMenu(e.clientX, e.clientY, id);
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
      <Show when={threadRootOf(props.id) === props.id}>
        <button type="button" class="comment-resolve" onMouseDown={e => e.preventDefault()} onClick={e => {
          e.stopPropagation(); setThreadResolved(props.id, !threadResolved(props.id));
        }}>{threadResolved(props.id) ? t("comment-reopen") : t("comment-resolve")}</button>
      </Show>
      <button type="button" class="comment-delete" title={t("comment-delete")} aria-label={t("comment-delete")} onClick={(e) => {
        e.stopPropagation();
        deleteComment(props.id);
      }}>×</button>
    </div>
  </Show>;
}

/** No document node exists while composing: abandoning an empty draft costs
 * zero writes. Once submitted it is an ordinary editable child block. */
export function CommentComposer(props: { id: string; surface: string; card?: () => HTMLElement | null | undefined }): JSX.Element {
  const { draft, setDraft, expanded, setExpanded } = commentDraftState(props.id, props.surface);
  // Hiding the margin would unmount this textarea and its unsent text.
  onCleanup(registerReplyDraft(() => draft(), props.id));
  let input!: HTMLTextAreaElement;
  const [focused, setFocused] = createSignal(false);
  const layerId = `comment-draft-${createUniqueId()}`;
  onMount(() => createEffect(() => {
    const held = replyFocusRequest();
    if (!held || held.id !== props.id || held.surface !== null && held.surface !== props.surface) return;
    const writable = blockWritable(props.id);
    if (writable) setExpanded(true);
    // Let the expanded class render and the old editor finish unmounting.
    queueMicrotask(() => {
      const target = writable ? input : props.card?.();
      if (replyFocusRequest() !== held || !target?.isConnected) return;
      if (!writable) target.tabIndex = -1;
      target.focus({ preventScroll: held.preventScroll });
      consumeReplyFocus();
    });
  }));
  const cancel = () => { setDraft(""); setExpanded(false); input.blur(); };
  createEffect(() => {
    if (!focused() || !expanded()) return;
    onCleanup(registerTransientLayer({ id: layerId, root: () => input.parentElement, dismiss: () => { cancel(); return true; } }));
  });
  // The first reply of a person with no name asks for one first (tine#841); the
  // draft stays in the composer while the dialog is open.
  const submit = () => { if (draft().trim()) runWithCommentName(() => { if (submitReply(props.id, draft())) cancel(); }); };
  return <Show when={blockWritable(props.id)}>
    <div class="comment-composer" classList={{ expanded: expanded() }} onMouseDown={e => e.stopPropagation()} onClick={e => e.stopPropagation()}>
      <textarea ref={input} class="comment-reply-draft" data-reply-root={props.id} rows={expanded() ? 3 : 1}
        placeholder={t("comment-reply-placeholder")} aria-label={t("comment-reply-placeholder")}
        value={draft()} onInput={e => setDraft(e.currentTarget.value)}
        onFocus={() => { setFocused(true); setExpanded(true); }}
        onBlur={e => { setFocused(false); if (!draft().trim() && !e.currentTarget.parentElement?.contains(e.relatedTarget as Node | null)) setExpanded(false); }}
        onKeyDown={e => {
          e.stopPropagation();
          if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "r") {
            e.preventDefault();
            const { selectionStart, selectionEnd } = input;
            if (draft().trim()) runWithCommentName(() => { if (commentFromReplyDraft(props.id, draft(), selectionStart, selectionEnd, props.surface)) cancel(); });
          }
          else if (e.key === "Escape") { e.preventDefault(); cancel(); }
          else if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.isComposing && e.keyCode !== 229) { e.preventDefault(); submit(); }
        }} />
      <Show when={expanded()}><DialogActions class="comment-composer-footer"
        cancel={<button type="button" onClick={cancel}>{t("confirm-cancel")}</button>}
        primary={<button type="button" class="comment-reply" disabled={!draft().trim()} onClick={submit}>{t("comment-send")}</button>}
      /></Show>
    </div>
  </Show>;
}

function sameRanges(a: readonly QuotedRange[], b: readonly QuotedRange[]): boolean {
  return a.length === b.length && a.every(([start, end, id], i) => start === b[i][0] && end === b[i][1] && id === b[i][2]);
}

function MountedRendering(props: { coverage?: QuoteCoverage }): JSX.Element {
  props.coverage?.rendering();
  return null;
}

/** Mark the passages `parentId`'s comment children quote inside the rendering
 * `children()` draws; a block with no children (`when` false) has none to mark
 * and draws it plainly, as does a hidden margin (GH #845). */
export function QuoteHighlightProvider(props: { parentId: string; format: Format; when: boolean; comments?: Accessor<CommentChildren>; coverage?: QuoteCoverage; children: () => JSX.Element }): JSX.Element {
  return <Show when={props.when && !commentMarginHidden()} fallback={props.children()}>
    <QuotedRendering {...props} />
  </Show>;
}

function QuotedRendering(props: Parameters<typeof QuoteHighlightProvider>[0]): JSX.Element {
  // Equal ranges keep the old array, so typing in a child does not re-render
  // the parent's text runs. The block's own comment producer is shared.
  const observedRanges = createQuotedSourceRanges(() => props.parentId, () => props.format, props.comments);
  const ranges = createMemo(observedRanges, [], { equals: sameRanges });
  // The renderer reports what it marks; the entry point reads that (`QuoteCoverage`).
  const coverage = props.coverage;
  return (
    <>
      <MountedRendering coverage={coverage} />
      <QuoteHighlightContext.Provider value={{ blockId: props.parentId, ranges, drew: coverage?.drew }}>
        {props.children()}
      </QuoteHighlightContext.Provider>
    </>
  );
}

export { MarginMarker } from "./CommentEntry";
