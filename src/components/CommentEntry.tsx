// Annotation entry points for the pane's tier (D-20261107-MC3 §§6–7,
// D-20261108-MC7/MC8). Medium (and the "In a popup" display at every width):
// hovering a highlight opens its thread through the hover preview's own trigger,
// delay and close rules (`createPeekBridge`); a mouse click places the caret; a
// touch tap or Enter on the focused highlight opens it. No rail and no count: only
// open threads the renderer drew no highlight for (quoted text changed,
// ambiguous, whole block, inside code or across markup) put a marker on their
// block. Phones: highlight or chip (no count) opens the bottom sheet. Only
// mounted annotated rows own observers. No document writes or additional source
// loads; page review owns grouping in a scope, standalone views group on open,
// and only the selected passage's threads are mounted.
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, useContext, type Accessor, type JSX } from "solid-js";
import { Block, SurfaceContext } from "./Block";
import { MarginContext, type MarginApi } from "./marginContext";
import { CommentGroup } from "./CommentGroup";
import { PeekPopup } from "../render/PeekPopup";
import { createPeekBridge } from "../render/peekBridge";
import type { QuoteCoverage } from "../render/quoteHighlight";
import { commentTier } from "../margin";
import { groupCommentEntries } from "../commentGroups";
import { node, formatForBlock, threadRootOf } from "../document";
import { editingId, editingSurface, endEdit } from "../editorController";
import { replyFocusRequest } from "../commentReplyFocus";
import { newResizeObserver, requestFrame, viewportOf } from "../windowRealm";
import { t } from "../i18n";
import { commentDraftState } from "../commentDraftState";
import { registerReplyDraft } from "../commentMargin";
import { useResolvedComments } from "./CommentResolutionScope";
import { NO_COMMENT_CHILDREN, type CommentChildren } from "../commentChildren";
import { useCommentReview } from "../commentReview";

export function MarginMarker(props: { parentId: string; comments: readonly string[]; source?: Accessor<CommentChildren>; coverage?: QuoteCoverage }): JSX.Element {
  return <Show when={props.comments.length > 0}><CommentEntry {...props} source={props.source ?? (() => NO_COMMENT_CHILDREN)} /></Show>;
}

const HIGHLIGHT = ".comment-quote-anchor[data-comment-id]";
const sameIds = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((id, i) => id === b[i]);
const CommentGlyph = () => <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 11.5a8.5 8.5 0 0 1-8.5 8.5H4l-3 3V11.5a10 10 0 0 1 20 0Z" /></svg>;

function CommentEntry(props: { parentId: string; comments: readonly string[]; source: Accessor<CommentChildren>; coverage?: QuoteCoverage }): JSX.Element {
  useResolvedComments();
  const place = useContext(MarginContext);
  const surface = useContext(SurfaceContext);
  const review = useCommentReview();
  const secondary = place?.surface !== surface && (surface.startsWith("ref:") || surface.startsWith("embed:"));
  const wide = () => !!place && place.thread === null && !secondary && place.api.active();
  let host!: HTMLSpanElement;
  let popupContent: HTMLDivElement | undefined;
  let row: HTMLElement | null = null;
  const [width, setWidth] = createSignal(800);
  const phone = () => commentTier(width()) === "phone";
  const format = () => formatForBlock(props.parentId);
  const entries = createMemo(() => {
    const available = new Set(props.comments);
    return props.source().entries.filter(entry => available.has(entry.id));
  });
  const openComments = createMemo(() => entries().filter(entry => !entry.resolved).map(entry => entry.id), [], { equals: sameIds });
  const resolvedComments = createMemo(() => entries().filter(entry => entry.resolved).map(entry => entry.id), [], { equals: sameIds });
  // Medium only: open threads the renderer drew no highlight for keep a block
  // marker. One answer (review round 2, AR-1): the coverage the renderer
  // reported, never a re-derivation from the source. While the block is edited
  // nothing is rendered, so the last answer stays (minus closed threads).
  const unanchored = createMemo<string[]>(previous => {
    if (wide() || phone()) return [];
    const open = openComments();
    const drawn = props.coverage?.drawn();
    if (!drawn) return previous.filter(id => open.includes(id));
    return open.filter(id => !drawn.has(id));
  }, [], { equals: sameIds });
  // One bridge per block: hover arms it, explicit entry (tap, Enter, marker) holds it open.
  const peek = createPeekBridge(wide);
  const [target, setTarget] = createSignal<{ id: string; anchor: HTMLElement }>();
  const opened = () => (peek.open() ? target() : undefined);
  const groups = createMemo(() => opened()
    ? review?.groups(props.parentId) ?? groupCommentEntries(node(props.parentId)?.raw ?? "", entries(), format())
    : []);
  // Indexed lookups (review round 3, PC-1): which group holds each thread.
  const presented = createMemo(() => new Set(props.comments));
  const groupOf = createMemo(() => {
    const at = new Map<string, number>();
    groups().forEach((ids, i) => { for (const id of ids) at.set(id, i); });
    return at;
  });
  const selectedId = () => presented().has(opened()?.id ?? "") ? opened()?.id : groups()[0]?.[0];
  const index = createMemo(() => groupOf().get(selectedId() ?? "") ?? 0);
  // The arrow keys walk exactly the threads this view presents (review finding 2).
  const visibleOrder = createMemo(() => groups()[index()] ?? [], [], { equals: sameIds });
  const openLabel = () => t("comment-margin-count", { count: openComments().length });
  const resolvedLabel = () => t("comment-resolved-count", { count: resolvedComments().length });
  const label = () => selectedId() && !openComments().includes(selectedId()!) ? resolvedLabel() : openLabel();
  const open = (id: string, anchor: HTMLElement) => { setTarget({ id, anchor }); peek.hold(); };
  // With no chip or rail (MC8) the entry span is empty: requests anchor the
  // popover below the commented block's text so it never covers that text.
  const blockText = () => (row ?? host.closest<HTMLElement>(".block-main"))?.querySelector<HTMLElement>(":scope > .block-content-wrapper") ?? host;
  const finishThreadEdit = () => {
    const id = editingId();
    if (id && (editingSurface() === null || editingSurface() === surface) && presented().has(threadRootOf(id, formatForBlock(props.parentId)) ?? "")) endEdit("blur");
    const active = host.ownerDocument.activeElement as HTMLElement | null;
    if (active?.closest(".comment-thread-popup")) active.blur();
  };
  const close = () => { finishThreadEdit(); peek.dismiss(); host.querySelector<HTMLButtonElement>(".margin-marker,.comment-resolved-entry")?.focus({ preventScroll: true }); };
  const move = (step: number) => {
    if (review) { finishThreadEdit(); review.move(step as 1 | -1, selectedId()); return; }
    const ids = groups()[index() + step];
    if (ids && opened()) { finishThreadEdit(); open(ids[0], opened()!.anchor); }
  };
  const api: MarginApi = place?.api ?? {
    active: () => false, register: () => () => {}, schedule: () => {}, activeComment: () => null, setActiveComment: () => {},
  };
  // Compact tiers make each highlight a keyboard target; the wide margin does not.
  // The wide margin re-measures this row (coalesced) when its text moves.
  let frame: (() => void) | undefined;
  const refresh = () => {
    frame = undefined;
    if (!row) return;
    for (const mark of row.querySelectorAll<HTMLElement>(HIGHLIGHT)) {
      if (!wide()) { mark.tabIndex = 0; mark.setAttribute("role", "button"); mark.setAttribute("aria-label", t("comment-open-thread")); }
      else { mark.removeAttribute("tabindex"); mark.removeAttribute("role"); mark.removeAttribute("aria-label"); }
    }
    api.schedule();
  };
  const schedule = () => { if (!frame) frame = requestFrame(host, refresh); };
  onCleanup(() => frame?.());
  onMount(() => {
    row = host.closest<HTMLElement>(".block-main");
    if (!row) return;
    if (review && !secondary) onCleanup(review.registerRow(props.parentId, row, surface));
    if (place && !secondary) onCleanup(api.register(props.parentId, row, props.source));
    const pane = row.closest<HTMLElement>(".comment-surface, [data-sidebar-surface], .live-ref-group, .main-content, .right-sidebar") ?? row.parentElement!;
    const read = () => { setWidth(pane.clientWidth || viewportOf(pane).width); schedule(); };
    read();
    const observer = newResizeObserver(pane, read); observer?.observe(pane); observer?.observe(row);
    onCleanup(() => observer?.disconnect());
    const fonts = host.ownerDocument.fonts;
    fonts?.addEventListener?.("loadingdone", schedule);
    onCleanup(() => fonts?.removeEventListener?.("loadingdone", schedule));
    const Observer = (row.ownerDocument.defaultView as (Window & typeof globalThis) | null)?.MutationObserver;
    const watcher = Observer ? new Observer(schedule) : null;
    watcher?.observe(row.querySelector(".block-content-wrapper") ?? row, { childList: true, subtree: true, characterData: true });
    onCleanup(() => watcher?.disconnect());
    const markOf = (event: Event) => {
      const mark = (event.target as Element | null)?.closest?.<HTMLElement>(HIGHLIGHT);
      return mark && presented().has(mark.dataset.commentId!) ? mark : null;
    };
    // Mouse: hover opens, click places the caret. Touch and pen: tap opens.
    let pointer = "mouse";
    const pointerDown = (event: PointerEvent) => {
      pointer = event.pointerType || "mouse";
      // A mouse press on a highlight is caret placement: cancel the dwell it armed.
      if (pointer === "mouse" && !wide() && !phone() && markOf(event)) peek.dismiss();
    };
    const tapOpens = () => phone() || pointer !== "mouse";
    const down = (event: MouseEvent) => { if (!wide() && tapOpens() && markOf(event)) event.stopPropagation(); };
    const click = (event: MouseEvent | KeyboardEvent) => {
      const mark = markOf(event);
      if (wide() || !mark) return;
      const keyboard = event instanceof host.ownerDocument.defaultView!.KeyboardEvent;
      if (keyboard ? event.key !== "Enter" && event.key !== " " : !tapOpens()) return;
      if (!keyboard && host.ownerDocument.defaultView?.getSelection()?.toString()) return;
      event.preventDefault(); event.stopPropagation(); open(mark.dataset.commentId!, mark);
    };
    const over = (event: PointerEvent) => {
      const mark = markOf(event);
      if (wide() || phone() || !mark || (event.relatedTarget as Node | null) && mark.contains(event.relatedTarget as Node)) return;
      peek.anchorEnter(event, () => {
        const popup = opened() ? popupContent?.closest(".comment-thread-popup") : null;
        if (popup?.contains(host.ownerDocument.activeElement) ||
          [...popup?.querySelectorAll<HTMLTextAreaElement>(".comment-reply-draft") ?? []].some(draft => draft.value.length > 0)) return;
        setTarget({ id: mark.dataset.commentId!, anchor: mark });
      });
    };
    const out = (event: PointerEvent) => {
      const mark = markOf(event);
      if (!mark || (event.relatedTarget as Node | null) && mark.contains(event.relatedTarget as Node)) return;
      peek.anchorLeave();
    };
    row.addEventListener("pointerdown", pointerDown, true); row.addEventListener("mousedown", down, true);
    row.addEventListener("click", click, true); row.addEventListener("keydown", click, true);
    row.addEventListener("pointerover", over); row.addEventListener("pointerout", out);
    onCleanup(() => {
      row?.removeEventListener("pointerdown", pointerDown, true); row?.removeEventListener("mousedown", down, true);
      row?.removeEventListener("click", click, true); row?.removeEventListener("keydown", click, true);
      row?.removeEventListener("pointerover", over); row?.removeEventListener("pointerout", out);
    });
  });
  createEffect(() => { void node(props.parentId)?.raw; props.comments; wide(); schedule(); });
  createEffect(() => { if (wide()) peek.dismiss(); });
  // Creation and Comment-on-existing requests must reveal the live thread even
  // when the old body editor has already unmounted. Preserve its surface key.
  createEffect(() => {
    if (wide()) return;
    const request = replyFocusRequest();
    const edit = editingId();
    const id = request && (request.surface === null || request.surface === surface) ? request.id
      : edit && (editingSurface() === null || editingSurface() === surface) ? threadRootOf(edit, formatForBlock(props.parentId)) : null;
    if (id && presented().has(id) && opened()?.id !== id) open(id, blockText());
    if (request?.preventScroll && opened() && !presented().has(request.id)) peek.dismiss();
  });
  let touch: { x: number; y: number } | undefined;
  const content = () => <div class="comment-popup-content" ref={popupContent}
    onFocusIn={event => {
      const target = event.target as HTMLElement;
      const block = target.closest<HTMLElement>("[data-block-id]")?.dataset.blockId;
      const id = target.closest<HTMLElement>("[data-reply-root]")?.dataset.replyRoot ?? (block && threadRootOf(block)) ?? selectedId();
      if (id) {
        const current = opened();
        if (current && groupOf().get(id) === index() && current.id !== id) setTarget({ ...current, id });
        review?.focus(id);
      }
    }}
    onTouchStart={event => { const first = event.touches[0]; touch = first && !(event.target as Element).closest("textarea,[contenteditable=true]") ? { x: first.clientX, y: first.clientY } : undefined; }}
    onTouchEnd={event => {
      const last = event.changedTouches[0];
      if (phone() && touch && last) {
        const dx = last.clientX - touch.x, dy = last.clientY - touch.y;
        if (Math.abs(dx) > 50 && Math.abs(dx) > Math.abs(dy) * 1.5) move(dx < 0 ? 1 : -1);
      }
      touch = undefined;
    }}>
    <div class="comment-popup-toolbar">
      <span>{review ? t("comment-review-position", { current: review.order().indexOf(selectedId() ?? "") + 1, count: review.order().length }) : t("comment-thread-position", { current: index() + 1, count: groups().length })}</span>
      <button type="button" class="comment-prev-thread" disabled={review ? !review.canMove(-1, selectedId()) : index() === 0} aria-label={t(review ? "command-comment-previous" : "comment-prev-thread")} onClick={() => move(-1)}>‹</button>
      <button type="button" class="comment-next-thread" disabled={review ? !review.canMove(1, selectedId()) : index() === groups().length - 1} aria-label={t(review ? "command-comment-next" : "comment-next-thread")} onClick={() => move(1)}>›</button>
      <button type="button" class="comment-popup-close" aria-label={t("comment-close-threads")} onClick={close}>×</button>
    </div>
    <SurfaceContext.Provider value={surface}>
      {/* Only the selected passage's threads are mounted (review round 3,
          PC-4/PC-9); unsent drafts outlive their composers through the leases
          this entry holds below (`commentDraftState`). */}
      <Show when={visibleOrder()[0]} keyed>{_key =>
        <CommentGroup ids={visibleOrder()} render={id => <MarginContext.Provider value={{ api, thread: { root: id, parent: props.parentId, order: visibleOrder } }}><Block id={id} /></MarginContext.Provider>} />}</Show>
    </SurfaceContext.Provider>
  </div>;
  return <span class="comment-entry" ref={host}>
    <For each={props.comments}>{id => {
      const state = commentDraftState(id, surface);
      onCleanup(registerReplyDraft(state.draft, id));
      return null;
    }}</For>
    <Show when={!wide()}>
      <Show when={phone() ? openComments().length > 0 : unanchored().length > 0}>
        <button type="button" class="margin-marker" classList={{ "comment-unanchored-marker": !phone() }}
          title={phone() ? openLabel() : t("comment-unanchored-marker", { count: unanchored().length })}
          aria-label={phone() ? openLabel() : t("comment-unanchored-marker", { count: unanchored().length })}
          onMouseDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); open((phone() ? openComments() : unanchored())[0], e.currentTarget); }}><CommentGlyph /></button>
      </Show>
      <Show when={resolvedComments().length > 0}><button type="button" class="comment-resolved-entry" title={resolvedLabel()} aria-label={resolvedLabel()}
        onMouseDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); open(resolvedComments()[0], e.currentTarget); }}>{resolvedLabel()}</button></Show>
      <Show when={opened()}><PeekPopup anchor={() => opened()?.anchor} blocks={() => []}
        class={`comment-thread-popup${phone() ? " comment-bottom-sheet" : ""}`} label={label()} bottomSheet={phone()}
        onPointerEnter={peek.popupEnter} onPointerLeave={peek.popupLeave} onDismiss={close} onHold={peek.setHeld} content={content()} /></Show>
    </Show>
  </span>;
}
