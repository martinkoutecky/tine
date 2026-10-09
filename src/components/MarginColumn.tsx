// Margin dialogue, slice 2 (vision 2026-10 §3.7 "Layout"): the comment column.
// On a wide main pane, a page that holds comments shifts its outline left and
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

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, untrack, type JSX } from "solid-js";
import { Block } from "./Block";
import { MarginContext, type MarginApi, type MarginPlacement } from "./marginContext";
import { countMarginMeasurePass, isCommentId, MARGIN_MIN_PANE_WIDTH, pageHasComment, stackThreads } from "../margin";
import { node as docNode, type FeedPage } from "../document";
import { newResizeObserver, requestFrame } from "../windowRealm";

export interface MarginSurface {
  /** Comments are drawn in the margin (the outline carries `margin-active`). */
  active: () => boolean;
  /** For the main column's blocks. */
  placement: MarginPlacement;
  /** The column, rendered after the section's outline. */
  Column: () => JSX.Element;
}

interface Thread { el: HTMLElement; parentId: string; row: HTMLElement }

/** The margin for one page section. `eligible` says whether this surface may
 * have a margin at all (the main pane's single-page view); the pane width and
 * whether the page holds a comment decide the rest. `outline` is the section's
 * `.page-blocks`; the column renders as its next sibling. */
export function createMarginSurface(opts: {
  page: () => FeedPage | undefined;
  eligible: () => boolean;
  outline: () => HTMLElement | undefined;
}): MarginSurface {
  const hasComment = createMemo(() => {
    if (!opts.eligible()) return false;
    const page = opts.page();
    return !!page && pageHasComment(page.roots, page.format);
  });
  // The pane's width (its scroller), watched only while the page has comments.
  const [paneWidth, setPaneWidth] = createSignal(0);
  createEffect(() => {
    if (!hasComment()) return;
    const outline = opts.outline();
    const pane = outline?.closest<HTMLElement>(".main-content") ?? outline?.parentElement?.parentElement;
    if (!pane) return;
    const read = () => setPaneWidth(pane.clientWidth);
    read();
    const observer = newResizeObserver(pane, read);
    observer?.observe(pane);
    onCleanup(() => observer?.disconnect());
  });
  const active = createMemo(() => hasComment() && paneWidth() >= MARGIN_MIN_PANE_WIDTH);

  const [rows, setRows] = createSignal<readonly { parentId: string; row: HTMLElement }[]>([]);
  const [activeComment, setActiveComment] = createSignal<string | null>(null);
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
    const items: { id: string; t: Thread; anchor: number; height: number; index: number }[] = [];
    for (const [id, t] of threads) {
      if (!t.el.isConnected || !t.row.isConnected) continue;
      const rowTop = t.row.getBoundingClientRect().top;
      const mark = marksIn(t.row).get(id);
      let anchor = rowTop;
      if (mark) {
        anchor = mark.getBoundingClientRect().top;
        offsets.set(id, anchor - rowTop);
      } else if (t.row.querySelector("textarea")) {
        anchor = rowTop + (offsets.get(id) ?? 0);
      }
      const index = untrack(() => docNode(t.parentId)?.children.indexOf(id) ?? 0);
      items.push({ id, t, anchor: anchor - origin, height: t.el.offsetHeight, index });
    }
    // Document order: commented blocks top to bottom, a block's comments in
    // child order (a stale or whole-block thread keeps its place among them).
    items.sort((a, b) => a.t.row === b.t.row ? a.index - b.index
      : a.t.row.compareDocumentPosition(b.t.row) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1);
    const tops = stackThreads(items);
    items.forEach((item, i) => { item.t.el.style.top = `${Math.round(tops[i])}px`; });
  };

  const api: MarginApi = {
    active,
    register(parentId, row) {
      setRows((list) => [...list, { parentId, row }]);
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

  function ThreadView(props: { id: string; parentId: string; row: HTMLElement }): JSX.Element {
    let el!: HTMLDivElement;
    onMount(() => {
      threads.set(props.id, { el, parentId: props.parentId, row: props.row });
      observer?.observe(el);
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
        classList={{ active: activeComment() === props.id }}
        data-thread-id={props.id}
        ref={el}
        onMouseEnter={() => setActiveComment(props.id)}
        onMouseLeave={() => setActiveComment(null)}
        onFocusIn={() => setActiveComment(props.id)}
        onFocusOut={(e) => { if (!el.contains(e.relatedTarget as Node | null)) setActiveComment(null); }}
      >
        <MarginContext.Provider value={{ api, thread: { root: props.id, parent: props.parentId } }}>
          <Block id={props.id} />
        </MarginContext.Provider>
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
      // A web font that finishes loading moves a passage inside its line
      // without resizing anything observed.
      const fonts = el.ownerDocument.fonts as FontFaceSet | undefined;
      const onFonts = () => api.schedule();
      fonts?.addEventListener?.("loadingdone", onFonts);
      api.schedule();
      onCleanup(() => {
        fonts?.removeEventListener?.("loadingdone", onFonts);
        observer?.disconnect();
        observer = null;
        if (column === el) column = undefined;
      });
    });
    createEffect(() => {
      rows();
      api.schedule();
    });
    const commentsOf = (parentId: string) => {
      const n = docNode(parentId);
      const format = opts.page()?.format ?? "md";
      return n ? n.children.filter((id) => isCommentId(id, format)) : [];
    };
    return (
      <aside class="margin-column" aria-label="Comments" ref={el}>
        <For each={rows()}>
          {(entry) => (
            <For each={commentsOf(entry.parentId)}>
              {(id) => <ThreadView id={id} parentId={entry.parentId} row={entry.row} />}
            </For>
          )}
        </For>
      </aside>
    );
  }

  return {
    active,
    placement: { api, thread: null },
    Column: () => <Show when={active()}><ColumnView /></Show>,
  };
}
