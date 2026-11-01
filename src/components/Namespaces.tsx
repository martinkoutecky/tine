import { For, Show, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { t } from "../i18n";
import type { PaneRouter } from "../router";
import { allPageNames, allPages } from "../pages";
import { installPageIndex } from "../pageIndex";
import { openPageContextMenu, openPageInSidebar } from "../ui";
import { openRouteInOtherPane } from "../panes";
import { pageIcon } from "../pageIconBatch";
import { PageIcon } from "./PageIcon";
import { ReferenceDisclosure } from "./ReferenceDisclosure";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { newResizeObserver } from "../windowRealm";
import { namespaceTreeFor } from "./Namespace";
import { visibleRange } from "../allPagesList";
import {
  allExpanded,
  defaultExpanded,
  indexNamespaces,
  matchingEntries,
  namespaceTotals,
  pathPrefix,
  visibleEntries,
} from "../namespacesList";

// The full Namespaces view (GH #335, D-20261101-L5). It is the All pages view's
// sibling: a route in the main pane, a fixed-height windowed list (only the visible
// slice is in the DOM, so a graph with tens of thousands of namespaced pages costs one
// screenful), a name filter. Its data is the sidebar's: the namespace tree over the
// shared page-name inventory (`namespaceTreeFor`), with no backend read at all.
// While a filter is typed the list is flat: every matching entry, in tree order, shown
// with its full path, so two `Notes` under different parents can be told apart.
// Multi-select delete is deliberately not offered (Martin, 2026-11-01).

/** Every row is exactly this tall, so the scroll offset maps to a row by arithmetic. */
export const NAMESPACES_ROW_HEIGHT = 32;
/** Rows deeper than this keep indenting no further, so a very deep import stays on screen. */
const MAX_INDENT_LEVELS = 24;
const INDENT_PX = 18;

export function Namespaces(props: { router: PaneRouter }): JSX.Element {
  installPageIndex();
  const [filter, setFilter] = createSignal("");
  // null = the default (top-level namespaces open); set by the first toggle, expand all or collapse all.
  const [expanded, setExpanded] = createSignal<ReadonlySet<string> | null>(null);
  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewport, setViewport] = createSignal(640);
  let scroller: HTMLDivElement | undefined;

  const entries = createMemo(() => indexNamespaces(namespaceTreeFor(allPageNames())));
  const totals = createMemo(() => namespaceTotals(entries()));
  const open = createMemo(() => expanded() ?? defaultExpanded(entries()));
  const filtering = () => filter().trim() !== "";
  const matches = createMemo(() => (filtering() ? matchingEntries(entries(), filter()) : []));
  const shown = createMemo(() => (filtering() ? matches() : visibleEntries(entries(), open())));
  const range = createMemo(() => visibleRange(scrollTop(), viewport(), NAMESPACES_ROW_HEIGHT, shown().length));
  const slice = createMemo(() => shown().slice(range().start, range().end));
  const loaded = () => allPages() !== undefined;

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
  const toggle = (full: string) => {
    const next = new Set(open());
    if (!next.delete(full)) next.add(full);
    setExpanded(next);
  };

  const openPage = (name: string, event: MouseEvent) => {
    const dest = internalLinkDest(event);
    if (dest === "sidebar") openPageInSidebar(name, "page");
    else if (dest === "background") props.router.openPageTargetInNewTab({ name, pageKind: "page" });
    else if (dest === "pane") void openRouteInOtherPane({ kind: "page", name, pageKind: "page" });
    else props.router.openPageTarget({ name, pageKind: "page" });
  };

  return (
    <div class="all-pages ns-view">
      <h1 class="page-title">{t("sidebar-namespaces-title")}</h1>
      <div class="all-pages-controls">
        <input
          class="all-pages-filter"
          type="search"
          placeholder={t("pages-namespaces-filter")}
          aria-label={t("pages-namespaces-filter")}
          value={filter()}
          onInput={(event) => { setFilter(event.currentTarget.value); resetScroll(); }}
        />
        <button type="button" class="ns-view-action" disabled={filtering()} onClick={() => { setExpanded(allExpanded(entries())); resetScroll(); }}>
          {t("pages-namespaces-expand-all")}
        </button>
        <button type="button" class="ns-view-action" disabled={filtering()} onClick={() => { setExpanded(new Set<string>()); resetScroll(); }}>
          {t("pages-namespaces-collapse-all")}
        </button>
        <span class="all-pages-count" aria-live="polite">
          <Show when={loaded()} fallback={t("pages-all-loading")}>
            {filtering()
              ? t("pages-namespaces-matches", { count: matches().length })
              : t("pages-namespaces-count", { pages: totals().pages, namespaces: totals().namespaces })}
          </Show>
        </span>
      </div>
      <div class="all-pages-table" role="table" aria-label={t("sidebar-namespaces-title")}>
        <div class="all-pages-head ns-view-grid" role="row">
          <span class="all-pages-col" role="columnheader">{t("pages-namespaces-column-namespace")}</span>
          <span class="all-pages-col ns-view-count-col" role="columnheader">{t("pages-namespaces-column-pages")}</span>
        </div>
        <div class="all-pages-scroll" ref={scroller} onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}>
          <div class="all-pages-spacer" role="tree" style={{ height: `${shown().length * NAMESPACES_ROW_HEIGHT}px` }}>
            <div class="all-pages-window" style={{ transform: `translateY(${range().start * NAMESPACES_ROW_HEIGHT}px)` }}>
              <For each={slice()}>
                {(index) => {
                  const entry = () => entries()[index];
                  const hasChildren = () => entry().node.children.length > 0;
                  const name = () => entry().node.full;
                  return (
                    <div
                      class="all-pages-row ns-view-grid ns-view-row"
                      role="treeitem"
                      aria-level={entry().depth + 1}
                      aria-expanded={!filtering() && hasChildren() ? open().has(name()) : undefined}
                      style={{ height: `${NAMESPACES_ROW_HEIGHT}px` }}
                      onMouseDown={internalLinkMouseDown}
                      onContextMenu={(event) => {
                        event.preventDefault();
                        openPageContextMenu(event.clientX, event.clientY, { name: name(), pageKind: "page" });
                      }}
                    >
                      <div
                        class="ns-view-name"
                        style={{ "padding-inline-start": filtering() ? "0" : `${Math.min(entry().depth, MAX_INDENT_LEVELS) * INDENT_PX}px` }}
                      >
                        <Show when={!filtering()}>
                          <Show when={hasChildren()} fallback={<span class="ns-view-spacer" />}>
                            <button
                              type="button"
                              class="ns-view-toggle"
                              aria-label={open().has(name()) ? t("pages-namespaces-collapse", { name: name() }) : t("pages-namespaces-expand", { name: name() })}
                              onClick={() => toggle(name())}
                            >
                              <ReferenceDisclosure collapsed={!open().has(name())} />
                            </button>
                          </Show>
                        </Show>
                        <button
                          type="button"
                          class="all-pages-open ns-view-open"
                          title={name()}
                          onClick={(event) => openPage(name(), event)}
                          onAuxClick={(event) => internalLinkAuxClick(event, () => props.router.openPageTargetInNewTab({ name: name(), pageKind: "page" }))}
                        >
                          <Show when={pageIcon(name())}>{(icon) => <span class="all-pages-icon"><PageIcon value={icon()} name={name()} /> </span>}</Show>
                          <Show when={filtering() && pathPrefix(entry())}>
                            <span class="ns-view-prefix">{pathPrefix(entry())}</span>
                          </Show>
                          <span class="ns-view-seg">{entry().node.seg}</span>
                        </button>
                      </div>
                      <span class="ns-view-count" role="cell">{entry().count > 0 ? entry().count : ""}</span>
                    </div>
                  );
                }}
              </For>
            </div>
          </div>
          <Show when={loaded() && shown().length === 0}>
            <p class="all-pages-empty">{filtering() ? t("pages-namespaces-no-matches") : t("pages-namespaces-empty")}</p>
          </Show>
        </div>
      </div>
    </div>
  );
}
