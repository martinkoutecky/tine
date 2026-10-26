import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { t } from "../i18n";
import { graphOwner, latestOwner, readOwned } from "../owned";
import { dataRev } from "../graphSession";
import type { PaneRouter } from "../router";
import { allPages, pageListLabels } from "../pages";
import { installPageIndex } from "../pageIndex";
import { openPageContextMenu, openPageInSidebar } from "../ui";
import { openRouteInOtherPane } from "../panes";
import { pageIcon } from "../pageIconBatch";
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
// Backlink counts use a separate coalesced read while mounted. Created and bulk deletion are
// not offered here: file creation time is not reliable across platforms.

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
  const [backlinks, setBacklinks] = createSignal<ReadonlyMap<string, number>>(new Map());
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
    return pages ? arrangeAllPages(pages, filter(), sort(), labels(), modified(), backlinks()) : [];
  });
  const range = createMemo(() => visibleRange(scrollTop(), viewport(), ALL_PAGES_ROW_HEIGHT, rows().length));
  const slice = createMemo(() => rows().slice(range().start, range().end));

  const metricsScope = {};
  let countPending = false;
  let countInFlight = false;
  let countStarted = -Infinity;
  let countTimer: ReturnType<typeof setTimeout> | undefined;
  const refreshCounts = () => {
    if (!alive || countInFlight || countTimer !== undefined || !countPending) return;
    const wait = Math.max(0, countStarted + 1000 - Date.now());
    if (wait) {
      countTimer = setTimeout(() => { countTimer = undefined; refreshCounts(); }, wait);
      return;
    }
    countPending = false;
    countInFlight = true;
    countStarted = Date.now();
    const owner = graphOwner(() => alive);
    void readOwned(owner, backend().pageBacklinkCounts()).then(result => {
      if (result.kind === "current") setBacklinks(new Map(result.value));
    }).catch(() => {
      // Missing counts leave dashes; updated times stay independently available.
    }).finally(() => {
      countInFlight = false;
      refreshCounts();
    });
  };
  onCleanup(() => { if (countTimer !== undefined) clearTimeout(countTimer); });
  createEffect(on(dataRev, () => {
    countPending = true;
    refreshCounts();
    // One read per open or graph revision, never per row or presentation edit.
    void (async () => {
      try {
        const owner = latestOwner(metricsScope, "metrics", graphOwner(() => alive));
        const result = await readOwned(owner, backend().pageModified());
        if (result.kind === "current") {
          setModified(new Map(result.value.modified));
        }
      } catch {
        // No times: Updated shows dashes and the list still works.
      }
    })();
  }));
  onMount(() => {
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
      <h1 class="page-title">{t("pages-all-title")}</h1>
      <div class="all-pages-controls">
        <input
          class="all-pages-filter"
          type="search"
          placeholder={t("pages-all-filter")}
          aria-label={t("pages-all-filter")}
          value={filter()}
          onInput={(event) => { setFilter(event.currentTarget.value); resetScroll(); }}
        />
        <label class="all-pages-journals">
          <input
            type="checkbox"
            class="themed-checkbox"
            checked={journals()}
            onChange={(event) => {
              setJournals(event.currentTarget.checked);
              // The journal-date column exists only while journals are listed; a sort by it cannot outlive it.
              if (!event.currentTarget.checked && sort().key === "day") setSort(DEFAULT_SORT);
              resetScroll();
            }}
          />{" "}
          {t("pages-all-show-journals")}
        </label>
        <span class="all-pages-count" aria-live="polite">
          <Show when={listed()} fallback={t("pages-all-loading")}>
            {filter().trim()
              ? t("pages-all-filtered-count", { count: rows().length, total: listed()!.length })
              : t("pages-all-count", { count: listed()!.length })}
          </Show>
        </span>
      </div>
      <div class="all-pages-table" classList={{ "all-pages-table--day": journals() }} role="table" aria-label={t("pages-all-title")}>
        <div class="all-pages-head" role="row">
          <button type="button" class="all-pages-col" role="columnheader" aria-sort={ariaSort("title")} onClick={() => sortBy("title")}>
            {t("pages-all-column-page")}{arrow("title")}
          </button>
          <button type="button" class="all-pages-col all-pages-updated-col" role="columnheader" aria-sort={ariaSort("updated")} onClick={() => sortBy("updated")}>
            {t("pages-all-column-updated")}{arrow("updated")}
          </button>
          <button type="button" class="all-pages-col" role="columnheader" aria-sort={ariaSort("backlinks")} onClick={() => sortBy("backlinks")}>
            {t("query-calm-backlinks")}{arrow("backlinks")}
          </button>
          <Show when={journals()}>
            <button type="button" class="all-pages-col all-pages-day-col" role="columnheader" aria-sort={ariaSort("day")} onClick={() => sortBy("day")}>
              {t("pages-all-column-journal-date")}{arrow("day")}
            </button>
          </Show>
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
                      {/* OG all-pages table: `icon::` before the name (page.cljs :td.name). */}
                      <Show when={pageIcon(row.entry.name)}>{(icon) => <span class="all-pages-icon">{icon()} </span>}</Show>
                      {row.label}
                    </button>
                    <span class="all-pages-updated" role="cell">{formatUpdated(row.modified)}</span>
                    <span class="all-pages-backlinks" role="cell">{row.backlinks ?? "—"}</span>
                    <Show when={journals()}>
                      <span class="all-pages-day" role="cell">{formatPageDay(row.entry.date_key)}</span>
                    </Show>
                  </div>
                )}
              </For>
            </div>
          </div>
          <Show when={listed() && rows().length === 0}>
            <p class="all-pages-empty">{filter().trim() ? t("pages-all-no-matches") : t("pages-all-empty")}</p>
          </Show>
        </div>
      </div>
    </div>
  );
}
