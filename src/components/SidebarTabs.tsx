// The sidebar's Tabs section (GH #704; v1 D-20261018-W1, v2 D-20261019-VT1):
// the open tabs of the focused pane as a list at every width, so a window too
// narrow for the top bar's strip (a phone, a small window) can still see and
// switch tabs. v2 adds the tab tree: drag a row to reorder (the strip and the
// ∨ overview follow, they read the same array), drag right to nest under the
// row above, left to move out; a parent's triangle collapses its children here
// only; "Move out of group" on a nested row's context menu. The router owns the
// tree (src/tabTree.ts); this list only derives depth and what collapse hides.
import { For, Show, createMemo, createSignal, type JSX } from "solid-js";
import { t } from "../i18n";
import { EmojiText } from "../render/emoji";
import { focusedRouterOf } from "../panes";
import { MAIN_WINDOW_ID } from "../windowRealm";
import type { Tab } from "../router";
import { tabDepths } from "../tabTree";
import { openActionContextMenu } from "../ui";
import { closeOffered, tabFullTitle, tabTitle } from "./TabBar";
import { beginRowReorderDrag, rowReorderClickSuppressed, type RowDropTarget } from "./rowReorder";
import "../styles/sidebarTabs.css";

const [tabsSectionExpanded, setTabsSectionExpanded] = createSignal(true);

/** Nesting step in px; also one pointer "step" right when choosing a depth
 *  (the Favorites rule). */
const INDENT_PX = 16;

interface TabRow {
  tab: Tab;
  depth: number;
  hasChildren: boolean;
  /** A collapsed row hiding the active tab: it carries the active cue. */
  hidesActive: boolean;
}

/** Drawn rows in tree order, skipping what a collapsed tab hides. A row's
 *  index is its `data-row-index`. */
export function visibleTabRows(tabs: readonly Tab[], activeId: string | null = null): TabRow[] {
  const depths = tabDepths(tabs);
  const rows: TabRow[] = [];
  let hiddenBelow = Infinity; // rows deeper than this are inside a collapsed tab
  tabs.forEach((tab, i) => {
    const depth = depths[i];
    if (depth > hiddenBelow) {
      if (tab.id === activeId) rows[rows.length - 1].hidesActive = true; // the collapsed row above
      return;
    }
    hiddenBelow = Infinity;
    const hasChildren = depths[i + 1] !== undefined && depths[i + 1] > depth;
    rows.push({ tab, depth, hasChildren, hidesActive: false });
    if (tab.collapsed && hasChildren) hiddenBelow = depth;
  });
  return rows;
}

/** Resolve a drop at `target` for the row at `from` over the rows that remain
 *  once the dragged subtree is lifted out: the tab it lands before, and the
 *  depth (the dragged row's own depth plus one per indent step right of the
 *  grab, clamped by the outliner rule; never under a pinned tab). */
function resolveTabDrop(rows: TabRow[], from: number, target: RowDropTarget) {
  const origin = rows[from];
  let end = from + 1;
  while (end < rows.length && rows[end].depth > origin.depth) end++;
  const kept = rows.filter((_, i) => i < from || i >= end);
  const slot = target.index + (target.before ? 0 : 1);
  const restSlot = rows.slice(0, slot).filter((_, i) => i < from || i >= end).length;
  const above = kept[restSlot - 1];
  const below = kept[restSlot];
  const max = !origin.tab.pinned && above && !above.tab.pinned ? above.depth + 1 : 0;
  const min = !origin.tab.pinned && below && !below.tab.pinned ? Math.min(below.depth, max) : 0;
  const depth = Math.max(min, Math.min(origin.depth + Math.round(target.dx / INDENT_PX), max));
  return { beforeId: below?.tab.id ?? null, depth };
}

const [dropTarget, setDropTarget] = createSignal<RowDropTarget | null>(null);
const [dropDepth, setDropDepth] = createSignal(0);

export function SidebarTabs(props: { onActiveNavigationComplete?: () => void }): JSX.Element {
  // The focused pane of the main window; the sidebar exists only there.
  const router = () => focusedRouterOf(MAIN_WINDOW_ID);
  const rows = createMemo(() => visibleTabRows(router().tabs(), router().activeId()));
  // The disclosure column appears once anything is nested; a flat list looks as v1 did.
  const nesting = createMemo(() => router().tabs().some((tab) => tab.parent !== null));
  const startDrag = (from: number, event: PointerEvent) => {
    if ((event.target as HTMLElement | null)?.closest(".nav-tab-close, .nav-tab-toggle")) return;
    const snapshot = rows();
    const owner = router();
    beginRowReorderDrag(event, "#sidebar-tabs-list .nav-tab",
      (target) => {
        if (target) setDropDepth(resolveTabDrop(snapshot, from, target).depth);
        setDropTarget(target);
      },
      (target) => {
        const { beforeId, depth } = resolveTabDrop(snapshot, from, target);
        owner.moveTabInTree(snapshot[from].tab.id, beforeId, depth);
      },
      { touchHold: true, bandHit: true });
  };
  return (
    // One tab is not worth a section; it appears when there is something to
    // switch to (the top bar's strip also has its single tab, but it is free).
    <Show when={router().tabs().length > 1}>
      <div class="nav-section" data-sidebar-section="tabs">
        <button
          type="button"
          class="nav-section-header nav-section-toggle"
          data-sidebar-section-toggle="tabs"
          aria-expanded={tabsSectionExpanded()}
          aria-controls="sidebar-tabs-list"
          onClick={() => setTabsSectionExpanded((open) => !open)}
        >
          <span class="nav-toggle-caret" classList={{ open: tabsSectionExpanded() }}>▸</span>{t("tab-bar-open-tabs")}<span class="nav-section-count">{router().tabs().length}</span>
        </button>
        <Show when={tabsSectionExpanded()}>
          <div id="sidebar-tabs-list" role="list">
            <For each={rows()}>
              {(row, i) => {
                const tab = row.tab;
                const dropping = () => dropTarget()?.index === i();
                return (
                  <div
                    class="nav-page nav-tab"
                    classList={{
                      active: tab.id === router().activeId(),
                      pinned: tab.pinned,
                      "has-active-child": row.hidesActive,
                      "row-drop-before": dropping() && dropTarget()!.before,
                      "row-drop-after": dropping() && !dropTarget()!.before,
                    }}
                    role="listitem"
                    data-tab-id={tab.id}
                    data-row-index={i()}
                    data-tab-depth={row.depth}
                    style={{
                      ...(row.depth > 0 ? { "padding-left": `${8 + row.depth * INDENT_PX}px` } : {}),
                      ...(dropping() ? { "--tab-drop-indent": `${8 + dropDepth() * INDENT_PX}px` } : {}),
                    }}
                    onPointerDown={(event) => startDrag(i(), event)}
                    onContextMenu={(event) => {
                      // "Move out of group": the non-drag path (keyboard, accessibility).
                      if (tab.parent === null) return;
                      event.preventDefault();
                      openActionContextMenu(event.clientX, event.clientY, [
                        { label: t("tab-bar-move-out-of-group"), run: () => router().moveTabOutOfGroup(tab.id) },
                      ]);
                    }}
                    onAuxClick={(event) => {
                      if (event.button !== 1) return;
                      event.preventDefault();
                      void router().closeTab(tab.id);
                    }}
                  >
                    <Show when={nesting()}>
                      <Show when={row.hasChildren} fallback={<span class="nav-tab-spacer" />}>
                        <button
                          type="button"
                          class="nav-tab-toggle"
                          aria-expanded={!tab.collapsed}
                          aria-label={tab.collapsed ? t("tab-bar-expand-group") : t("tab-bar-collapse-group")}
                          onClick={() => router().setTabCollapsed(tab.id, !tab.collapsed)}
                        >
                          <span class="nav-toggle-caret" classList={{ open: !tab.collapsed }}>▸</span>
                        </button>
                      </Show>
                    </Show>
                    <button
                      type="button"
                      class="nav-tab-main"
                      aria-current={tab.id === router().activeId() ? "true" : undefined}
                      title={String(tabFullTitle(router().tabRoute(tab)))}
                      onClick={() => {
                        if (rowReorderClickSuppressed()) return;
                        router().setActiveTab(tab.id);
                        props.onActiveNavigationComplete?.();
                      }}
                    >
                      <Show when={tab.pinned}><span class="nav-tab-pin"><EmojiText text="📌" /></span></Show>
                      <span class="nav-tab-title"><EmojiText text={tabTitle(router().tabRoute(tab))} /></span>
                    </button>
                    <Show when={closeOffered(router(), tab)}>
                      <button
                        type="button"
                        class="nav-tab-close"
                        title={t("tab-bar-close-tab")}
                        aria-label={t("tab-bar-close", { title: String(tabFullTitle(router().tabRoute(tab))) })}
                        onClick={(event) => {
                          event.stopPropagation();
                          void router().closeTab(tab.id);
                        }}
                      >✕</button>
                    </Show>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>
      </div>
    </Show>
  );
}
