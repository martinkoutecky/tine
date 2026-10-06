import { Show, createEffect, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { PaneEdgeHighlights, PaneSelectHint, PaneTree } from "../App";
import { TabBar } from "./TabBar";
import { FailureBoundary } from "./FailureBoundary";
import { WindowOverlays } from "./WindowOverlays";
import {
  currentLayoutWindowId,
  firstPaneId,
  focusedRouterOf,
  layoutHasMultiplePanes,
  layoutRoot,
  paneRouter,
  visibleLayoutNode,
} from "../panes";
import { routeTitle } from "../router";
import { dimInactiveBlocks, documentMode, openSwitcher, wideMode } from "../ui";
import { WindowContext, windowById } from "../windowRealm";

/**
 * A workspace window's whole UI (OG-MULTIWINDOW): its own pane tree with tabs,
 * back/forward on its focused pane, and the app overlays while the user is in
 * it. There is no left sidebar and no right sidebar in a workspace window.
 */
export function WorkspaceWindowShell(props: { windowId: string }): JSX.Element {
  const id = props.windowId;
  const router = () => focusedRouterOf(id);
  const here = () => currentLayoutWindowId() === id;
  createEffect(() => {
    const title = routeTitle(router().route());
    const win = windowById(id);
    if (win) win.document.title = `${title} — Tine`;
  });
  return (
    <WindowContext.Provider value={id}>
      <div
        class="app-container workspace-window"
        data-workspace-window={id}
        classList={{
          "sidebar-collapsed": true,
          "wide-mode": wideMode(),
          "document-mode": documentMode(),
          "dim-mode": dimInactiveBlocks(),
        }}
      >
        <div class="main-container">
          <header class="topbar workspace-window-topbar">
            <div class="topbar-left">
              <button class="icon-btn" title="Search (Ctrl+K)" aria-label="Search" data-search-trigger data-pane-focus-neutral onClick={() => openSwitcher()}>
                <svg viewBox="0 0 24 24" class="nav-icon">
                  <circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" stroke-width="1.7" />
                  <line x1="16.5" y1="16.5" x2="21" y2="21" stroke="currentColor" stroke-width="1.7" />
                </svg>
              </button>
              <button class="icon-btn topbar-navigation-action" title="Go back" data-pane-focus-neutral disabled={!router().canGoBack()} onClick={() => router().goBack()}>
                <svg viewBox="0 0 24 24" class="nav-icon">
                  <path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </button>
              <button class="icon-btn topbar-navigation-action" title="Go forward" data-pane-focus-neutral disabled={!router().canGoForward()} onClick={() => router().goForward()}>
                <svg viewBox="0 0 24 24" class="nav-icon">
                  <path d="M9 5l7 7-7 7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </button>
            </div>
            <Show when={!layoutHasMultiplePanes(layoutRoot(id))} fallback={<div class="topbar-spacer" />}>
              {/* Keyed on the sole pane's id: TabBar freezes its router at mount. */}
              <Show when={firstPaneId(layoutRoot(id))} keyed>
                {(soloId) => <FailureBoundary region="The tabs"><TabBar router={paneRouter(soloId)} /></FailureBoundary>}
              </Show>
            </Show>
          </header>
          <div class="content-row">
            <div class="drawer-workspace">
              <Show when={here()}>
                <PaneEdgeHighlights />
                <PaneSelectHint />
              </Show>
              <PaneTree node={visibleLayoutNode(id)} path={[]} />
            </div>
          </div>
        </div>
        <WindowOverlays windowId={id} />
      </div>
    </WindowContext.Provider>
  );
}

/** The renderer src/workspaceWindows.ts installs (via main.tsx). */
export function renderWorkspaceWindowShell(windowId: string, mount: HTMLElement): () => void {
  return render(() => <WorkspaceWindowShell windowId={windowId} />, mount);
}
