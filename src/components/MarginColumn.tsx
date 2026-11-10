// Margin dialogue, slice 2 (vision 2026-10 §3.7 "Layout"): the comment column.
// In the wide tier, a page that holds comments shifts its outline left and
// draws each comment thread in a right-hand column, top-aligned to the passage
// it quotes (or to the commented block when the quote is stale or whole-block).
// Threads that would overlap stack downward in document order; nothing overlaps.
//
// Cost (D-10): the column draws only threads whose commented block is mounted
// (a windowed outline unmounts far-away rows, and their markers with them), and
// one measurement pass reads O(drawn threads) rectangles, then writes their
// offsets. Passes are coalesced to one per frame and run only when something
// that can move a thread changed: the section or a thread resized (one
// ResizeObserver), a marker mounted or unmounted, or a commented block's text
// or one of its comments changed. Typing in any other block that keeps its
// height costs nothing; one that grows costs one pass.

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, untrack, useContext, type Accessor, type JSX } from "solid-js";
import { Block } from "./Block";
import { t } from "../i18n";
import { MarginContext, type MarginApi, type MarginPlacement } from "./marginContext";
import { countMarginMeasurePass, commentTier, marginColumnWidth, stackThreads } from "../margin";
import { CommentIndexContext } from "../commentIndex";
import { node, type FeedPage } from "../document";
import { commentMarginHidden } from "../commentMarginPref";
import { newResizeObserver, requestFrame } from "../windowRealm";
import { groupCommentEntries, threadOrder } from "../commentGroups";
import type { CommentChildren } from "../commentChildren";
import { CommentGroup } from "./CommentGroup";
import { useResolvedComments } from "./CommentResolutionScope";
import { threadResolved } from "../commentResolution";
import { resolvedCommentAnchorTop } from "./resolvedCommentAnchor";

export interface MarginSurface {
  /** Comments are drawn in the margin (the outline carries `margin-active`). */
  active: () => boolean;
  /** For the main column's blocks. */
  placement: MarginPlacement;
  /** The column, rendered after the section's outline. */
  Column: () => JSX.Element;
}

interface Thread { el: HTMLElement; parentId: string; row: HTMLElement }

// Journal days share one reading column. Releasing a day's reservation must
// leave the width needed by the remaining days, including during feed eviction.
const columnWidths = new WeakMap<HTMLElement, Map<HTMLElement, number>>();
function reserveColumnWidth(inner: HTMLElement, outline: HTMLElement, width: number): () => void {
  let widths = columnWidths.get(inner);
  if (!widths) columnWidths.set(inner, widths = new Map());
  widths.set(outline, width);
  inner.style.setProperty("--margin-column-width", `${width}px`);
  return () => {
    widths.delete(outline);
    const remaining = [...widths.values()].at(-1);
    if (remaining === undefined) {
      columnWidths.delete(inner);
      inner.style.removeProperty("--margin-column-width");
    } else inner.style.setProperty("--margin-column-width", `${remaining}px`);
  };
}

/** The margin for one page section. `eligible` says whether this surface may
 * have a margin at all; the viewer's tier policy and whether the page holds
 * a comment decide the rest. `outline` is the section's
 * `.page-blocks`; the column renders as its next sibling. */
export function createMarginSurface(opts: {
  page: () => FeedPage | undefined;
  eligible: () => boolean;
  outline: () => HTMLElement | undefined;
  pane?: () => HTMLElement | undefined;
}): MarginSurface {
  const [hasComment, setHasComment] = createSignal(false);
  // The pane's width (its scroller), watched only while the page has comments.
  const [paneWidth, setPaneWidth] = createSignal(0);
  createEffect(() => {
    if (!hasComment()) return;
    const outline = opts.outline();
    const pane = opts.pane?.() ?? outline?.closest<HTMLElement>(".main-content") ?? outline?.parentElement?.parentElement;
    if (!pane) return;
    const read = () => setPaneWidth(pane.clientWidth);
    read();
    const observer = newResizeObserver(pane, read);
    observer?.observe(pane);
    onCleanup(() => observer?.disconnect());
  });
  const active = createMemo(() => hasComment() && commentTier(paneWidth()) === "wide");
  // A pane with room to spare beside the reading column gives it to the threads
  // (Martin 2026-10-31): the column grows from 280 px up to a readable maximum,
  // never taking width from the outline.
  createEffect(() => {
    const width = paneWidth();
    if (!active()) return;
    const outline = opts.outline();
    const inner = opts.pane?.() ?? outline?.closest<HTMLElement>(".main-content-inner");
    if (!inner || !outline) return;
    const style = inner.ownerDocument.defaultView?.getComputedStyle(inner);
    const content = style ? parseFloat(style.getPropertyValue("--tine-main-content-max-width") || style.getPropertyValue("--ls-main-content-max-width")) : NaN;
    const wide = !!inner.closest(".wide-mode");
    const column = marginColumnWidth(width, wide || !Number.isFinite(content) ? null : content);
    onCleanup(reserveColumnWidth(inner, outline, column));
  });

  const [rows, setRows] = createSignal<readonly { parentId: string; row: HTMLElement; comments: Accessor<CommentChildren> }[]>([]);
  const [activeComment, setActiveComment] = createSignal<string | null>(null);
  const [focusedComment, setFocusedComment] = createSignal<string | null>(null);
  const threads = new Map<string, Thread>();
  // Where a quoted passage sat inside its block row at the last pass: while that
  // block is being edited its marks are gone, so its threads stay put.
  const offsets = new Map<string, number>();
  let column: HTMLElement | undefined;
  let observer: ResizeObserver | null = null;
  let cancelFrame: (() => void) | null = null;

  const measure = () => {
    if (!column?.isConnected || threads.size === 0) return;
    countMarginMeasurePass();
    const origin = column.getBoundingClientRect().top;
    const marksByRow = new Map<HTMLElement, Map<string, HTMLElement>>();
    const marksIn = (row: HTMLElement) => {
      let marks = marksByRow.get(row);
      if (!marks) {
        marks = new Map();
        for (const mark of row.querySelectorAll<HTMLElement>(".comment-quote-anchor[data-comment-id]")) {
          const id = mark.dataset.commentId!;
          if (!marks.has(id)) marks.set(id, mark);
        }
        marksByRow.set(row, marks);
      }
      return marks;
    };
    const items: { id: string; t: Thread; anchor: number; height: number }[] = [];
    for (const [id, t] of threads) {
      if (!t.el.isConnected || !t.row.isConnected) continue;
      const rowTop = t.row.getBoundingClientRect().top;
      const mark = marksIn(t.row).get(id);
      let anchor = rowTop;
      if (mark) {
        anchor = mark.getBoundingClientRect().top;
        offsets.set(id, anchor - rowTop);
      } else if (threadResolved(id) && !t.row.classList.contains("editing")) {
        anchor = resolvedCommentAnchorTop(id, t.parentId, t.row) ?? rowTop;
      } else if (t.row.classList.contains("editing")) {
        // Any editor (the textarea, or the CodeMirror editor, which has none)
        // drops the marks: keep the thread where its passage last was, clamped
        // to the row (MARGIN-POS, Martin 2026-10-31: threads jumped to the
        // block's top on entering edit and back on leaving it).
        anchor = rowTop + Math.min(offsets.get(id) ?? 0, Math.max(0, t.row.offsetHeight - 1));
      }
      items.push({ id, t, anchor: anchor - origin, height: t.el.offsetHeight });
    }
    // Blocks stay in document order; within each block use passage order,
    // retaining file order when passages tie.
    items.sort((a, b) => a.t.row === b.t.row ? (a.t.el.compareDocumentPosition(b.t.el) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1)
      : a.t.row.compareDocumentPosition(b.t.row) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1);
    const tops = stackThreads(items, undefined, items.findIndex(item => item.id === untrack(focusedComment)));
    items.forEach((item, i) => { item.t.el.style.top = `${Math.round(tops[i])}px`; });
    // A feed day's last thread can be taller than its outline. Reserve its
    // space so the next day's text and margin start below this day's cards.
    const bottom = items.reduce((end, item, i) => Math.max(end, Math.round(tops[i]) + item.height), 0);
    column.parentElement?.style.setProperty("--margin-content-bottom", `${Math.ceil(bottom)}px`);
  };

  const api: MarginApi = {
    active,
    register(parentId, row, comments) {
      setRows((list) => [...list, { parentId, row, comments }]);
      return () => setRows((list) => list.filter((entry) => entry.row !== row || entry.parentId !== parentId));
    },
    schedule() {
      if (cancelFrame || !column) return;
      cancelFrame = requestFrame(column, () => {
        cancelFrame = null;
        measure();
      });
    },
    activeComment,
    setActiveComment,
  };
  onCleanup(() => cancelFrame?.());
  createEffect(() => { focusedComment(); api.schedule(); });

  // Emphasis both ways: the passage of the hovered or focused thread, and the
  // thread of the hovered passage.
  createEffect(() => {
    const outline = opts.outline();
    if (!active() || !outline) return;
    const id = activeComment();
    for (const mark of outline.querySelectorAll<HTMLElement>(".comment-quote-anchor.active")) mark.classList.remove("active");
    if (id) for (const mark of outline.querySelectorAll<HTMLElement>(".comment-quote-anchor[data-comment-id]")) {
      if (mark.dataset.commentId === id) mark.classList.add("active");
    }
  });
  createEffect(() => {
    const outline = opts.outline();
    if (!active() || !outline) return;
    const markOf = (target: EventTarget | null) =>
      (target as Element | null)?.closest?.<HTMLElement>(".comment-quote-anchor[data-comment-id]") ?? null;
    const over = (e: MouseEvent) => {
      const mark = markOf(e.target);
      if (mark) setActiveComment(mark.dataset.commentId!);
    };
    const out = (e: MouseEvent) => {
      const mark = markOf(e.target);
      if (mark && markOf(e.relatedTarget) !== mark) setActiveComment(null);
    };
    outline.addEventListener("mouseover", over);
    outline.addEventListener("mouseout", out);
    onCleanup(() => {
      outline.removeEventListener("mouseover", over);
      outline.removeEventListener("mouseout", out);
    });
  });

  function ThreadView(props: { id: string; ids: readonly string[]; parentId: string; row: HTMLElement; order: () => readonly string[] }): JSX.Element {
    let el!: HTMLDivElement;
    onMount(() => {
      threads.set(props.id, { el, parentId: props.parentId, row: props.row });
      observer?.observe(el);
      // A newly focused comment must already sit beside its passage before
      // paint or caret reveal. Existing threads still coalesce resize/edit work.
      measure();
      api.schedule();
      onCleanup(() => {
        if (threads.get(props.id)?.el === el) threads.delete(props.id);
        observer?.unobserve(el);
        api.schedule();
      });
    });
    return (
      <div
        class="margin-thread"
        classList={{ active: props.ids.includes(activeComment() ?? "") }}
        data-thread-id={props.id}
        ref={el}
        onMouseEnter={() => setActiveComment(props.id)}
        onMouseLeave={() => setActiveComment(null)}
        onFocusIn={() => { setActiveComment(props.id); setFocusedComment(props.id); }}
        onFocusOut={(e) => { if (!el.contains(e.relatedTarget as Node | null)) { setActiveComment(null); setFocusedComment(null); } }}
      >
        <CommentGroup ids={props.ids} render={id => <MarginContext.Provider value={{ api, thread: { root: id, parent: props.parentId, order: props.order } }}><Block id={id} /></MarginContext.Provider>} />
      </div>
    );
  }

  function ColumnView(): JSX.Element {
    let el!: HTMLElement;
    onMount(() => {
      column = el;
      observer = newResizeObserver(el, () => api.schedule());
      // The outline (edits, collapse, width) and the whole section (a title
      // that rewraps moves the outline without resizing it).
      const outline = opts.outline();
      if (outline) observer?.observe(outline);
      if (outline?.parentElement) observer?.observe(outline.parentElement);
      for (const t of threads.values()) observer?.observe(t.el);
      // Initial ThreadView mounts precede the column's onMount.
      measure();
      // A web font that finishes loading moves a passage inside its line
      // without resizing anything observed.
      const fonts = el.ownerDocument.fonts as FontFaceSet | undefined;
      const onFonts = () => api.schedule();
      fonts?.addEventListener?.("loadingdone", onFonts);
      api.schedule();
      onCleanup(() => {
        fonts?.removeEventListener?.("loadingdone", onFonts);
        el.parentElement?.style.removeProperty("--margin-content-bottom");
        observer?.disconnect();
        observer = null;
        if (column === el) column = undefined;
      });
    });
    createEffect(() => {
      rows();
      api.schedule();
    });
    function RowGroups(props: { parentId: string; row: HTMLElement; comments: Accessor<CommentChildren> }): JSX.Element {
      const showResolved = useResolvedComments();
      // One surface order for the cards and the arrow keys (review finding 2).
      const ordered = createMemo(() => groupCommentEntries(node(props.parentId)?.raw ?? "",
        props.comments().entries.filter(entry => showResolved() || !entry.resolved), opts.page()?.format ?? "md"));
      const groups = createMemo(() => new Map(ordered().map(ids => [ids[0], ids])));
      const order = createMemo(() => threadOrder(ordered()));
      return <For each={[...groups().keys()]}>{id => <ThreadView id={id} ids={groups().get(id)!} parentId={props.parentId} row={props.row} order={order} />}</For>;
    }
    return (
      <aside class="margin-column" aria-label={t("comment-margin-label")} ref={el}>
        <For each={rows()}>
          {(entry) => <RowGroups parentId={entry.parentId} row={entry.row} comments={entry.comments} />}
        </For>
      </aside>
    );
  }

  return {
    active,
    placement: { api, thread: null },
    Column: () => {
      const index = useContext(CommentIndexContext);
      createEffect(() => setHasComment(opts.eligible() && !commentMarginHidden() && !!index?.roots().length));
      return <Show when={active()}><ColumnView /></Show>;
    },
  };
}
