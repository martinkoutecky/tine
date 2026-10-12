import { For, Show, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { graphOwner, readOwned } from "../owned";
import type { PaneRouter } from "../router";
import { allPages, pageListLabels } from "../pages";
import { installPageIndex } from "../pageIndex";
import { openPageContextMenu, openPageInSidebar } from "../ui";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { newResizeObserver } from "../windowRealm";
import {
  DEFAULT_SORT,
  arrangeAllPages,
  formatPageDay,
  formatUpdated,
  listablePages,
  nextSort,
  visibleRange,
  type AllPagesRow,
  type AllPagesSort,
  type AllPagesSortKey,
  type ModifiedTimes,
} from "../allPagesList";

// The full All Pages view (GH #335). OG: frontend/components/page.cljs `all-pages`
// lists name / backlinks / created / updated, sortable per column, with a name
// search and a journals toggle (off by default). The rows are the shared page
// index (`pageIndex.ts`); the Updated column is ONE `page_modified` read when the
// view opens (times the store already holds), never a call per row. Only the
// visible slice is in the DOM, so 10k+ pages cost what one screenful costs.
// Created and backlink-count columns, bulk actions (delete, orphan removal) are
// not here: file creation time is not reliable across platforms.

/** Every row is exactly this tall, so the scroll offset maps to a row by arithmetic. */
export const ALL_PAGES_ROW_HEIGHT = 32;

export function AllPages(props: { router: PaneRouter }): JSX.Element {
  installPageIndex();
  const [filter, setFilter] = createSignal("");
  const [journals, setJournals] = createSignal(false);
  const [sort, setSort] = createSignal<AllPagesSort>(DEFAULT_SORT);
  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewport, setViewport] = createSignal(640);
  const [modified, setModified] = createSignal<ModifiedTimes>(new Map());
  let scroller: HTMLDivElement | undefined;
  let alive = true;
  onCleanup(() => { alive = false; });

  const listed = createMemo(() => {
    const pages = allPages();
    return pages ? listablePages(pages, journals()) : undefined;
  });
  const labels = createMemo(() => pageListLabels(listed() ?? []));
  const rows = createMemo<AllPagesRow[]>(() => {
    const pages = listed();
    return pages ? arrangeAllPages(pages, filter(), sort(), labels(), modified()) : [];
  });
  const range = createMemo(() => visibleRange(scrollTop(), viewport(), ALL_PAGES_ROW_HEIGHT, rows().length));
  const slice = createMemo(() => rows().slice(range().start, range().end));

  onMount(() => {
    // One read per open. The list is usable meanwhile; Updated fills in when it lands.
    void (async () => {
      try {
        const result = await readOwned(graphOwner(() => alive), backend().pageModified());
        if (result.kind === "current") setModified(new Map(result.value.modified));
      } catch {
        // No times: Updated shows dashes and the list still works.
      }
    })();
    const measure = () => { if (scroller?.isConnected) setViewport(scroller.clientHeight || 640); };
    measure();
    const observer = scroller ? newResizeObserver(scroller, measure) : null;
    observer?.observe(scroller!);
    onCleanup(() => observer?.disconnect());
  });

  const resetScroll = () => {
    setScrollTop(0);
    if (scroller) scroller.scrollTop = 0;
  };
  const sortBy = (key: AllPagesSortKey) => {
    setSort((current) => nextSort(current, key));
    resetScroll();
  };
  const ariaSort = (key: AllPagesSortKey): "ascending" | "descending" | "none" =>
    sort().key !== key ? "none" : sort().desc ? "descending" : "ascending";
  const arrow = (key: AllPagesSortKey) => (sort().key !== key ? "" : sort().desc ? " ▾" : " ▴");

  const open = (row: AllPagesRow, event: MouseEvent) => {
    const { entry } = row;
    const target = { name: entry.name, pageKind: entry.kind, path: entry.path };
    const dest = internalLinkDest(event);
    if (dest === "sidebar") openPageInSidebar(target);
    else if (dest === "background") props.router.openPageTargetInNewTab(target);
    else if (dest === "pane") void openRouteInOtherPane({ kind: "page", ...target });
    else props.router.openPageTarget(target);
  };

  return (
    <div class="all-pages">
      <h1 class="page-title">All pages</h1>
      <div class="all-pages-controls">
        <input
          class="all-pages-filter"
          type="search"
          placeholder="Filter page names"
          aria-label="Filter page names"
          value={filter()}
          onInput={(event) => { setFilter(event.currentTarget.value); resetScroll(); }}
        />
        <label class="all-pages-journals">
          <input
            type="checkbox"
            checked={journals()}
            onChange={(event) => { setJournals(event.currentTarget.checked); resetScroll(); }}
          />{" "}
          Show journals
        </label>
        <span class="all-pages-count" aria-live="polite">
          <Show when={listed()} fallback="Loading…">
            {filter().trim() ? `${rows().length} of ${listed()!.length}` : listed()!.length} pages
          </Show>
        </span>
      </div>
      <div class="all-pages-table" role="table" aria-label="All pages">
        <div class="all-pages-head" role="row">
          <button type="button" class="all-pages-col" role="columnheader" aria-sort={ariaSort("title")} onClick={() => sortBy("title")}>
            Page{arrow("title")}
          </button>
          <button type="button" class="all-pages-col all-pages-updated-col" role="columnheader" aria-sort={ariaSort("updated")} onClick={() => sortBy("updated")}>
            Updated{arrow("updated")}
          </button>
          <button type="button" class="all-pages-col all-pages-day-col" role="columnheader" aria-sort={ariaSort("day")} onClick={() => sortBy("day")}>
            Journal date{arrow("day")}
          </button>
        </div>
        <div
          class="all-pages-scroll"
          ref={scroller}
          onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        >
          <div class="all-pages-spacer" style={{ height: `${rows().length * ALL_PAGES_ROW_HEIGHT}px` }}>
            <div class="all-pages-window" style={{ transform: `translateY(${range().start * ALL_PAGES_ROW_HEIGHT}px)` }}>
              <For each={slice()}>
                {(row) => (
                  <div
                    class="all-pages-row"
                    role="row"
                    style={{ height: `${ALL_PAGES_ROW_HEIGHT}px` }}
                    onMouseDown={internalLinkMouseDown}
                    onContextMenu={(event) => {
                      event.preventDefault();
                      openPageContextMenu(event.clientX, event.clientY, { name: row.entry.name, pageKind: row.entry.kind, path: row.entry.path });
                    }}
                  >
                    <button
                      type="button"
                      class="all-pages-open"
                      role="cell"
                      title={row.entry.path}
                      onClick={(event) => open(row, event)}
                      onAuxClick={(event) =>
                        internalLinkAuxClick(event, () => props.router.openPageTargetInNewTab({ name: row.entry.name, pageKind: row.entry.kind, path: row.entry.path }))}
                    >
                      {row.label}
                    </button>
                    <span class="all-pages-updated" role="cell">{formatUpdated(row.modified)}</span>
                    <span class="all-pages-day" role="cell">{formatPageDay(row.entry.date_key)}</span>
                  </div>
                )}
              </For>
            </div>
          </div>
          <Show when={listed() && rows().length === 0}>
            <p class="all-pages-empty">{filter().trim() ? "No page names match." : "No pages yet."}</p>
          </Show>
        </div>
      </div>
    </div>
  );
}
