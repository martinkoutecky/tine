import { For, Show, createEffect, createResource, createSignal, onCleanup, type JSX } from "solid-js";
import { pageIconPrefix } from "../pageIconBatch";
import { msg, t } from "../i18n";
import { JournalsIcon } from "./JournalsIcon";
import { openAllPages, openConflicts, openJournals, openPage, openPageInNewTab, openInNewTab, openPageTarget, openPageTargetInNewTab, route, type PageTarget } from "../router";
import { openSwitcher, recentPages, openPageContextMenu, openActionContextMenu, openPageInSidebar, favoritesSectionExpanded, recentSectionExpanded, toggleFavoritesSection, toggleRecentSection } from "../ui";
import { pendingConflictCount } from "../liveConflicts";
import { graphMeta } from "../graphSession";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { pushToast } from "../toasts";
import { switchGraph, createNewGraph, loadGraphPath, authorizeGraphAccess, type LoadGraphPathOutcome } from "../graph";
import { reportGraphOpenFailure } from "../graphOpenFailure";
import { backend } from "../backend";
import { graphOwner, ownedWhen, readOwned, writeOwned } from "../owned";
import { allPageNames, navigationName } from "../pageIndex";
import { SidebarTitle } from "./SidebarTitle";
import { NamespaceTree } from "./Namespace";
import { SidebarFavorites } from "./SidebarFavorites";
import { favoriteCount } from "../favorites";
import type { PageKind } from "../types";
import { registerTransientLayer } from "../transientLayers";
import { writeClipboardText } from "../clipboard";
import { isMobilePlatform } from "../nativeChrome";
import { graphRowMenuActions } from "./graphRowMenu";
import { readOr } from "../resourceRead";
import { ResourceFailure } from "./ResourceFailure";
import { queryAllWindows } from "../windowRealm";

export function sidebarPageTarget(name: string, kind: PageKind): { name: string; kind: PageKind } {
  return { name: kind === "page" ? navigationName(name) : name, kind };
}

export interface SidebarPageOpenDeps {
  normal: (name: string, kind: PageKind) => void;
  sidebar: (name: string, kind: PageKind) => void;
  newTab: (name: string, kind: PageKind) => void;
  pane: (name: string, kind: PageKind) => void;
  context: (x: number, y: number, name: string, kind: PageKind) => void;
}

const sidebarPageOpenDeps: SidebarPageOpenDeps = {
  normal: openPage,
  sidebar: openPageInSidebar,
  newTab: openPageInNewTab,
  pane: (name, kind) => void openRouteInOtherPane({ kind: "page", name, pageKind: kind }),
  context: openPageContextMenu,
};

export function openSidebarPageTarget(
  name: string,
  kind: PageKind,
  gesture: "normal" | "sidebar" | "new-tab" | "pane" | "context",
  point: { x: number; y: number } = { x: 0, y: 0 },
  deps: SidebarPageOpenDeps = sidebarPageOpenDeps,
  onActiveNavigationComplete?: () => void,
) {
  const target = sidebarPageTarget(name, kind);
  if (gesture === "normal") { deps.normal(target.name, target.kind); onActiveNavigationComplete?.(); }
  else if (gesture === "sidebar") deps.sidebar(target.name, target.kind);
  else if (gesture === "new-tab") deps.newTab(target.name, target.kind);
  else if (gesture === "pane") deps.pane(target.name, target.kind);
  else deps.context(point.x, point.y, target.name, target.kind);
}

export interface GraphNavigationActions {
  openKnown(path: string, newWindow: boolean): Promise<LoadGraphPathOutcome>;
  openPicked(): Promise<LoadGraphPathOutcome>;
  createNew(): Promise<LoadGraphPathOutcome>;
}

const graphNavigationActions: GraphNavigationActions = {
  openKnown: openKnownGraph,
  openPicked: switchGraph,
  createNew: createNewGraph,
};

export function Sidebar(props: {
  onActiveNavigationComplete?: () => void;
  graphActions?: GraphNavigationActions;
} = {}): JSX.Element {
  const [showNs, setShowNs] = createSignal(false);

  // `path` disambiguates nested pages that share a basename (#21 Phase 2). It's
  // optional: favorites / recent are keyed by name only, so they match on name
  // regardless of which file-path variant is currently routed.
  const isActive = (name: string, path?: string) => {
    const r = route();
    return (
      r.kind === "page" &&
      r.name === name &&
      (path === undefined || !r.path || r.path === path)
    );
  };
  // Sidebar page rows follow the shared modified-click contract (linkGesture.ts):
  // Shift → right sidebar (GH #63), Ctrl/Cmd or middle → background tab, Alt →
  // other pane. The mousedown half suppresses shift-range selection (GH #42).
  const shiftGuard = internalLinkMouseDown;
  const openRowMenu = (e: MouseEvent, name: string, kind: PageKind) => {
    e.preventDefault();
    openPageContextMenu(e.clientX, e.clientY, name, kind);
  };

  return (
    <div class="left-sidebar-inner">
      <div class="sidebar-header">
        <div class="app-logo">{t("sidebar-tine")}</div>
        <GraphSwitcher
          onActiveNavigationComplete={props.onActiveNavigationComplete}
          actions={props.graphActions ?? graphNavigationActions}
        />
      </div>

      <div class="nav-contents">
        <div
          class="nav-item"
          classList={{ active: route().kind === "journals" }}
          onClick={() => { openJournals(); props.onActiveNavigationComplete?.(); }}
          onMouseDown={internalLinkMouseDown}
          onAuxClick={(e) => internalLinkAuxClick(e, () => openInNewTab({ kind: "journals" }))}
        >
          <JournalsIcon />
          <span>{t("sidebar-journals")}</span>
        </div>

        <Show when={favoriteCount() > 0}>
          <div class="nav-section">
            <button
              type="button"
              class="nav-section-header nav-section-toggle"
              data-sidebar-section="favorites"
              aria-expanded={favoritesSectionExpanded()}
              aria-controls="sidebar-favorites-list"
              onClick={toggleFavoritesSection}
            >
              <span class="nav-toggle-caret" classList={{ open: favoritesSectionExpanded() }}>▸</span>{t("sidebar-favorites")}<span class="nav-section-count">{favoriteCount()}</span>
            </button>
            <Show when={favoritesSectionExpanded()}>
              <SidebarFavorites
                onActiveNavigationComplete={props.onActiveNavigationComplete}
                isActive={(name) => isActive(name)}
                targetName={(name, kind) => sidebarPageTarget(name, kind).name}
                open={(name, kind, gesture, point) =>
                  openSidebarPageTarget(name, kind, gesture, point, sidebarPageOpenDeps, props.onActiveNavigationComplete)}
              />
            </Show>
          </div>
        </Show>

        <Show when={recentPages().length > 0}>
          <div class="nav-section">
            <button
              type="button"
              class="nav-section-header nav-section-toggle"
              data-sidebar-section="recent"
              aria-expanded={recentSectionExpanded()}
              aria-controls="sidebar-recent-list"
              onClick={toggleRecentSection}
            >
              <span class="nav-toggle-caret" classList={{ open: recentSectionExpanded() }}>▸</span>{t("sidebar-recent")}<span class="nav-section-count">{recentPages().length}</span>
            </button>
            <Show when={recentSectionExpanded()}>
              <div id="sidebar-recent-list">
                <For each={recentPages()}>
                  {(r) => {
                    const target = (): PageTarget => ({ name: r.name, pageKind: r.kind, ...(r.path ? { path: r.path } : {}) });
                    return (
                      <div
                        class="nav-page"
                        classList={{ active: isActive(target().name, target().path) }}
                        onMouseDown={shiftGuard}
                        onClick={(e) => {
                          const dest = internalLinkDest(e);
                          if (dest === "sidebar") openPageInSidebar(target());
                          else if (dest === "background") openPageTargetInNewTab(target());
                          else if (dest === "pane") openRouteInOtherPane({ kind: "page", ...target() });
                          else { openPageTarget(target()); props.onActiveNavigationComplete?.(); }
                        }}
                        onAuxClick={(e) => internalLinkAuxClick(e, () => openPageTargetInNewTab(target()))}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          openPageContextMenu(e.clientX, e.clientY, target());
                        }}
                      >
                        <SidebarTitle text={pageIconPrefix(r.name) + (r.name.startsWith("hls__") ? r.name.slice(5) : r.name)} fullTitle={r.name.startsWith("hls__") ? r.name.slice(5) : r.name} />
                      </div>
                    );
                  }}
                </For>
              </div>
            </Show>
          </div>
        </Show>

        <div class="nav-section">
          <button
            type="button"
            class="nav-section-header nav-all-pages-open"
            classList={{ active: route().kind === "all-pages" }}
            onClick={() => { openAllPages(); props.onActiveNavigationComplete?.(); }}
          >
            {t("pages-all-title")}
          </button>
        </div>

        <Show when={allPageNames().some((name) => name.includes("/"))}><div class="nav-section">
          <div
            class="nav-section-header nav-section-toggle"
            onClick={() => setShowNs(!showNs())}
          >
            <span class="nav-toggle-caret" classList={{ open: showNs() }}>▸</span>{t("sidebar-namespaces")}</div>
          <Show when={showNs()}>
            <NamespaceTree onPageContextMenu={openRowMenu} onActiveNavigationComplete={props.onActiveNavigationComplete} />
          </Show>
        </div></Show>
      </div>

      <div class="sidebar-footer">
        <ConflictQueueBadge onActiveNavigationComplete={props.onActiveNavigationComplete} />
        <button class="new-page-btn" onClick={() => openSwitcher()}>{t("sidebar-new-page")}</button>
      </div>
    </div>
  );
}

// Display name for the active graph = basename of its root folder (OG shows the
// same). Falls back to "No graph" when none is loaded (e.g. mock/first run).
function graphDisplayName(): string {
  const root = graphMeta()?.display_root ?? graphMeta()?.root;
  if (!root) return t("navigation-prose-no-graph");
  const base = root.replace(/[/\\]+$/, "").split(/[/\\]/).pop();
  return base || root;
}

export interface KnownGraphOpenDeps {
  switchInPlace(path: string): Promise<LoadGraphPathOutcome>;
  openNewWindow(path: string): Promise<LoadGraphPathOutcome>;
}

export function openKnownGraph(
  path: string,
  newWindow: boolean,
  deps: KnownGraphOpenDeps = {
    switchInPlace: loadGraphPath,
    openNewWindow: async (target) => {
      const owner = graphOwner();
      const authorized = await readOwned(owner, authorizeGraphAccess(target));
      if (authorized.kind === "stale" || !authorized.value) return { kind: "aborted" };
      // A graph opened in a peer window is never an in-place navigation even
      // when the backend had to construct that window, so it must keep the
      // mobile drawer open.
      const opened = await readOwned(owner, backend().openGraphWindow(target));
      if (opened.kind === "stale") return { kind: "aborted" };
      return { kind: "focused_existing" };
    },
  }
): Promise<LoadGraphPathOutcome> {
  return newWindow ? deps.openNewWindow(path) : deps.switchInPlace(path);
}

// The current-graph control in the sidebar header. OG puts a graph-name dropdown
// top-left (database icon → switch/new/all-graphs/re-index). Tine lists its known
// graphs here alongside open/create actions; Shift-click opens a peer window.
export function GraphSwitcher(props: {
  onActiveNavigationComplete?: () => void;
  actions: GraphNavigationActions;
}): JSX.Element {
  const [open, setOpen] = createSignal(false);
  let alive = true;
  onCleanup(() => { alive = false; });
  const [knownGraphsResource, { refetch }] = createResource(() => backend().listKnownGraphs());
  // A failed list never throws into render and never reads as "no other graphs":
  // the menu says it could not load them and offers Retry (ResourceFailure).
  const knownGraphs = () => readOr(knownGraphsResource, undefined, "known graphs");
  const close = () => setOpen(false);

  createEffect(() => {
    if (open()) void refetch();
  });

  createEffect(() => {
    if (!open()) return;
    const unregister = registerTransientLayer({
      id: "graph-switch-menu",
      root: () => queryAllWindows<HTMLElement>(".graph-switch-menu"),
      dismiss: () => { close(); return true; },
    });
    onCleanup(unregister);
  });

  return (
    <div class="graph-switch">
      <button
        class="graph-switch-btn"
        title={graphMeta()?.display_root ?? graphMeta()?.root ?? undefined}
        onClick={() => setOpen(!open())}
      >
        <svg viewBox="0 0 24 24" class="graph-switch-icon">
          <ellipse cx="12" cy="5" rx="7" ry="3" fill="none" stroke="currentColor" stroke-width="1.6" />
          <path d="M5 5v7c0 1.66 3.13 3 7 3s7-1.34 7-3V5" fill="none" stroke="currentColor" stroke-width="1.6" />
          <path d="M5 12v7c0 1.66 3.13 3 7 3s7-1.34 7-3v-7" fill="none" stroke="currentColor" stroke-width="1.6" />
        </svg>
        <span class="graph-switch-name">{graphDisplayName()}</span>
        <span class="graph-switch-caret">▾</span>
      </button>
      <Show when={open()}>
        <div
          class="graph-switch-backdrop"
          onClick={close}
          onContextMenu={(e) => {
            e.preventDefault();
            close();
          }}
        />
        <div class="ctx-menu graph-switch-menu">
          <ResourceFailure of={knownGraphsResource} what="your other graphs" onRetry={() => void refetch()} />
          <For each={knownGraphs() ?? []}>
            {(graph) => (
              <div
                class="ctx-item graph-switch-row"
                classList={{ active: graph.path === graphMeta()?.root }}
                title={graph.display_path ?? graph.path}
                onContextMenu={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  openActionContextMenu(event.clientX, event.clientY, graphRowMenuActions(graph, {
                    openKnown: props.actions.openKnown,
                    reveal: (path) => backend().revealKnownGraph(path),
                    copyPath: writeClipboardText,
                    forget: async (path) => {
                      const result = await writeOwned(ownedWhen(() => alive), backend().forgetKnownGraph(path));
                      if (result.kind === "current") await refetch();
                    },
                    desktop: !isMobilePlatform,
                    isCurrent: graph.path === graphMeta()?.root,
                  }));
                }}
                onClick={(event) => {
                  const newWindow = event.shiftKey;
                  close();
                  const attempt = () => void props.actions.openKnown(graph.path, newWindow)
                    .then((outcome) => {
                      if (!newWindow && (outcome.kind === "loaded" || outcome.kind === "already_current")) {
                        props.onActiveNavigationComplete?.();
                      }
                    })
                    .catch((error) => reportGraphOpenFailure(error, attempt));
                  attempt();
                }}
              >
                <span class="graph-switch-row-name">{graph.name}</span>
                <button
                  class="graph-switch-remove"
                  title={t("sidebar-remove-from-this-list", { name: String(graph.name) })}
                  aria-label={t("sidebar-remove-from-this-list", { name: String(graph.name) })}
                  onClick={(event) => {
                    event.stopPropagation();
                    void writeOwned(ownedWhen(() => alive), backend().forgetKnownGraph(graph.path))
                      .then((result) => { if (result.kind === "current") void refetch(); })
                      .catch((error) => pushToast(msg("sidebar-couldn-t-remove-graph", { error: String(error) }), "error"));
                  }}
                >
                  ×
                </button>
              </div>
            )}
          </For>
          <Show when={(knownGraphs() ?? []).length > 0}>
            <div class="ctx-separator" />
          </Show>
          <div
            class="ctx-item"
            onClick={() => {
              close();
              void props.actions.openPicked()
                .then((outcome) => {
                  if (outcome.kind === "loaded" || outcome.kind === "already_current") props.onActiveNavigationComplete?.();
                })
                // Target-specific failures are reported by switchGraph after the
                // picker returns a path, so their Retry keeps that path. A
                // rejection here is a picker failure with no target to retry.
                .catch((error) => pushToast(msg("sidebar-couldn-t-open-the-graph-picker", { error: String(error) }), "error"));
            }}
          >{t("sidebar-open-graph")}</div>
          <div
            class="ctx-item"
            onClick={() => {
              close();
              void props.actions.createNew()
                .then((outcome) => {
                  if (outcome.kind === "loaded" || outcome.kind === "already_current") props.onActiveNavigationComplete?.();
                })
                .catch((error) => pushToast(msg("sidebar-couldn-t-create-the-graph", { error: String(error) }), "error"));
            }}
          >{t("sidebar-new-graph")}</div>
        </div>
      </Show>
    </div>
  );
}

// Concord: the calm badge. A conflict is a persistent object, not an
// interruption: it waits here, never opens a modal, and survives restarts
// because the queue is derived from disk. It opens the Conflicts overview.
// A copy whose page is gone counts too, or nothing would point at it.
// On a phone the sidebar is a drawer over the page: like every other navigation
// row, opening the overview must close it, or the overview opens out of sight.
export function ConflictQueueBadge(props: { onActiveNavigationComplete?: () => void }): JSX.Element {
  const count = pendingConflictCount;
  return (
    <Show when={count()}>
      <button class="conflict-queue-badge" title={t("sidebar-review-the-pages-that-need-a-decision")} onClick={() => { openConflicts(); props.onActiveNavigationComplete?.(); }}>
        <span class="conflict-queue-dot" aria-hidden="true" />
        {t("sidebar-conflict-count", { count: count() })}
      </button>
    </Show>
  );
}
