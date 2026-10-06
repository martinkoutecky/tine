import { installTineLinks } from "./deepLinkNavigation";
import { DeepLinkGraphChoice } from "./components/DeepLinkGraphChoice";
import { resizeSidebar, commitSidebarWidth } from "./sidebarSizing";
import { Match, Show, Suspense, Switch, createEffect, createSignal, lazy, on, onCleanup, onMount, type JSX } from "solid-js";
import { Sidebar } from "./components/Sidebar";
import { PageView, reloadJournalsFeedFromStart, type JournalsFeedOwner } from "./components/Page";
import { QueryWorkspace } from "./components/QueryWorkspace";
import { ConflictOverview } from "./components/ConflictOverview";
// pdf.js (~hundreds of KB) is heavy and most sessions never open a PDF — load
// the viewer only when one is opened.
const KeyedPdfViewer = lazy(() =>
  import("./components/PdfViewer").then((m) => ({ default: m.KeyedPdfViewer }))
);
import { TabBar, tabDropHighlightsPane, tabSplitPreviewSideForPane } from "./components/TabBar";
import { WorkspaceSwitcher } from "./components/WorkspaceSwitcher";
import { TopbarOverflowMenu } from "./components/TopbarOverflowMenu";
import { WindowOverlays } from "./components/WindowOverlays";
import { closeAllWorkspaceWindows } from "./workspaceWindows";
import { CalendarJump } from "./components/CalendarJump";
import { ConflictBar } from "./components/ConflictBar";
import { installReloadOnFocus, refreshingFromDisk, subscribeWatcherFreshness, trackGraphChangeApplication } from "./reloadOnFocus";
import { subscribeAssetChanges } from "./assetRefresh";
import { initConflictPolicy } from "./conflictPolicy";
import { RightSidebar } from "./components/RightSidebar";
import { MobileKeyboardToolbar } from "./components/MobileKeyboardToolbar";
import {
  DrawerBackground,
  MobileDrawerController,
  MobileDrawerPanel,
  dismissDrawerAndRestore,
} from "./components/MobileDrawerShell";
import { InPageFind } from "./components/InPageFind";
import { installKeybindings } from "./keybindings";
import { installFileDrop } from "./filedrop";
import { installBlockSelectionDrag } from "./blockDrag";
import { applyGraphConfigChange, loadGraphPath, persistedGraphPath } from "./graph";
import { installPageIndex } from "./pageIndex";
import { scheduleAutomaticUpdateCheck, setUpdateExitGuard } from "./update";
import { WelcomeLayer } from "./components/Welcome";
import { FailureBoundary } from "./components/FailureBoundary";
import { goBack, goForward, canGoBack, canGoForward, flushSession, openJournals, openPage, sameRoute, type PaneRouter, type PdfRoute, type QueryRoute } from "./router";
import { theme, toggleTheme, sidebarOpen, toggleSidebar, rightSidebarOpen, toggleRightSidebar, openSwitcher, sidebarWidth, openSettings, welcomeOpen, closeWelcome, shortcutOverrides, wideMode, documentMode, focusMode, dimInactiveBlocks, exitFocusMode, installPaneTracker, refreshSyncConflicts, graphTransitioning, setGraphTransitioning, activeDrawer, completeActiveLeftNavigation, dismissMobileDrawer, setLeftSidebarOpen } from "./ui";
import { graphMeta, firstLoadDone, setFirstLoadDone, graphEpoch, setStartupOpenFailure } from "./graphSession";
import { applyGraphChange, installAliasDraftRouteHandler, installExternalChangeUiHandler } from "./document";

installAliasDraftRouteHandler((name, kind) => openPage(name, kind));
import { pushToast } from "./toasts";
import { mobileDrawerMode, restoreDrawerFocus } from "./mobileDrawers";
import { dismissTopTransient, topTransientLayer } from "./transientLayers";
import { applyZoom, installInterfaceZoomKeys, installInterfaceZoomWheel } from "./zoom";
import { flushAll, appendToTodayJournal, captureToPage, unsavedDrafts, unsavedPageCount } from "./document";
import type { QuickCaptureAck, QuickCaptureRequest } from "./quickCaptureAck";
import { backend, isTauri } from "./backend";
import { isPublishedExport, loadPublishedSnapshot } from "./publishedBackend";
import { openPublishedPermalink, publishedPermalinkForWorkspace, replacePublishedPermalink } from "./publishedPermalink";
import { maybeShowDefenderHint } from "./defenderHint";
import { bindingOwner, graphOwner, latestOwner, ownedWhen, readOwned, readOwnedResource, writeOwned, type Owned, type Owner, type WriteOwner } from "./owned";
import { parserFailed } from "./render/parse";
import { warnIfSoftwareRendering } from "./gpu";
import { initSmoothScroll } from "./smoothScroll";
import { initCopySettings } from "./copySettings";
import { initRefCompletionSettings } from "./refCompletionSettings";
import { initNavSettings } from "./navSettings";
import { initLocalFileSettings } from "./localFileSettings";
import { initAssetSettings } from "./assetSettings";
import { initMediaEditorSettings } from "./mediaEditorSettings";
import { initSpellcheckSettings } from "./spellcheckSettings";
import { initLinkDefault } from "./editor/linkDefault";
import { initDebug, dbg, recordDiagnostic, recordSessionActive } from "./debug";
import { WindowControls, ResizeGrips, installWindowChrome, maximized } from "./components/WindowChrome";
import { initNativeChrome, isMac, isMobilePlatform, osDrawsWindowControls, touchGesturePlatform } from "./nativeChrome";
import {
  PaneContext,
  closePane,
  currentLayoutWindowId,
  firstPaneId,
  focusedPaneId,
  layoutHasMultiplePanes,
  layoutRoot,
  visibleLayoutNode,
  paneRouter,
  openPdfNotes,
  layoutPaneIds,
  allPaneIds,
  setSplitRatio,
  type LayoutNode,
} from "./panes";
import { paneSel, samePaneTarget } from "./paneSelect";
import { SurfaceContext } from "./components/Block";
import { endEdit } from "./editorController";
import { createAndroidRootCloseCoordinator, exitAndroidActivity, installAndroidBackHandler } from "./androidBack";
import { appBackAvailable, dispatchAppBack } from "./appBack";
import { installEdgeSwipe } from "./edgeSwipe";
import { createSafeCloseCoordinator } from "./safeClose";
import { openUnsavedRecovery } from "./unsavedRecovery";
import { UnsavedRecovery } from "./components/UnsavedRecovery";
import { installDraftStore, writeAtRisk } from "./draftStore";
import { currentPdfOwnership, drainPdfWork } from "./pdfOwnership";
import { hlsPageName } from "./pdf";
import type { InvalidRoute } from "./routeTypes";
import { installBackgroundFlush } from "./backgroundFlush";
import { installSessionActivity } from "./sessionActivity";
import { initSettingsLayout } from "./settingsLayout";
import { initCodeDisplay } from "./codeDisplay";
import { initContentWidths } from "./contentWidth";
import { MAIN_WINDOW_ID, isElementNode, isHTMLElementNode, isNodeValue, mainWindow, newResizeObserver, registeredWindows, requestFrame, useWindowId, windowOf } from "./windowRealm";


/** The single persistence transaction used by both desktop close and Android
 * root Back.  Callers choose only the final platform action. */
export const safeClose = createSafeCloseCoordinator({
  blurActive() {
    // Every Tine window: an editor focused in a workspace popup commits too.
    for (const win of registeredWindows()) {
      const active = win.document.activeElement;
      if (isHTMLElementNode(active)) active.blur();
    }
  },
  endEdit() {
    endEdit("graph-switch");
  },
  flushPdfWork: drainPdfWork,
  flushAll,
  // GH #540: name the pages at risk; "No" opens the recovery panel.
  confirmDiscard: (reason) => {
    const explanation = reason === "still-saving"
      ? "Tine is still writing your changes and is taking longer than expected — a slow or network drive can do this."
      : "Tine has changes that could not be saved (a conflict or a stuck save).";
    const inventory = unsavedDrafts().map((p) => `• ${p.name} — ${p.state}`).join("\n") || "Pending attachments or storage work; no page draft identified.";
    return backend().confirm(
      `${explanation}\n\n${inventory}\n\nChoose No to review, retry saving, or copy your drafts. Close this window anyway and lose them?`,
      "Unsaved changes",
    );
  },
  onDiscardDeclined: openUnsavedRecovery,
  recordDiscard: (reason) => recordDiagnostic("close_discarded_unsaved", { closeReason: reason, pages: unsavedPageCount() }),
  // A close that keeps unsaved pages leaves their newest drafts in app data first.
  flushSession: () => writeAtRisk().then(flushSession),
  setTransition: setGraphTransitioning,
  notifyPdfFailure: () => {
    pushToast("Couldn't save pending PDF changes. The graph remains open.", "error");
  },
  notifyStillSaving: () => {
    pushToast("Still saving your changes — closing in a moment.", "info");
  },
  notifyConfirmationFailure: () => {
    pushToast("Couldn't confirm closing the window. Your unsaved changes are still open.", "error");
  },
});

setUpdateExitGuard(safeClose);

// Master parity (AndroidRootClosePhase): once the frontend close is accepted the
// graph is durable, a failed activity exit keeps the transition shield, and the
// next Back retries only the exit, never the flush.
const androidRootClose = createAndroidRootCloseCoordinator(safeClose, {
  finishActivity: exitAndroidActivity,
  finishActivityFailed: () => pushToast(
    "Tine couldn't close the Android activity. Tap Back to retry closing.",
    "error",
  ),
});

async function closeAndroidRootSafely(): Promise<void> {
  await androidRootClose.request();
}

/** Capture the actual live Journals surfaces that justified a watcher restart.
 * The shared feed may be displayed in either half of a split; a main-router
 * check alone would let an old graph/navigation response land in that feed. */
function journalsFeedOwner(
  routes: Array<{ paneId: string; route: ReturnType<PaneRouter["route"]> }>
): JournalsFeedOwner | null {
  const epoch = graphEpoch();
  const owners = routes.filter((p) => p.route.kind === "journals");
  if (!owners.length) return null;
  return {
    graphEpoch: epoch,
    isLive: () =>
      graphEpoch() === epoch && owners.some((p) =>
        allPaneIds().includes(p.paneId) && sameRoute(paneRouter(p.paneId).route(), p.route)
      ),
  };
}

function requestJournalFeedWatcherRestart(
  routes: Array<{ paneId: string; route: ReturnType<PaneRouter["route"]> }>
) {
  const owner = journalsFeedOwner(routes);
  if (owner) void reloadJournalsFeedFromStart(owner);
}

installExternalChangeUiHandler(() => {
  const routes = allPaneIds().map((paneId) => ({ paneId, router: paneRouter(paneId), route: paneRouter(paneId).route() }));
  return {
    pageOpen: (name: string) => routes.some((p) => p.route.kind === "page" && p.route.name === name),
    journalsOpen: routes.some((p) => p.route.kind === "journals"),
    leaveRemovedPage: (name: string) => {
      for (const p of routes) {
        if (p.route.kind === "page" && p.route.name === name) {
          if (p.router.canGoBack()) p.router.goBack();
          else if (!closePane(p.paneId)) p.router.openJournals({ inPlace: true });
        }
      }
    },
    restartJournalFeed: () => requestJournalFeedWatcherRestart(routes),
  };
});

export function PaneTree(props: { node: LayoutNode; path: number[] }): JSX.Element {
  const n = () => props.node;
  // Keyed leaf: PaneLeaf freezes its router (and its context providers) at
  // mount, so a leaf whose paneId changes IN PLACE (layout restore, sibling
  // collapse) must REMOUNT, not update — otherwise it keeps rendering the old
  // pane's router/tabs.
  const leafId = () => (n().kind === "pane" ? (n() as Extract<LayoutNode, { kind: "pane" }>).paneId : null);
  return (
    <Show
      when={n().kind === "split" ? (n() as Extract<LayoutNode, { kind: "split" }>) : null}
      fallback={
        <Show when={leafId()} keyed>
          {(id) => <PaneLeaf paneId={id} />}
        </Show>
      }
    >
      {(split) => {
        return (
          <div class={`pane-split pane-split-${split().dir}`}>
            <div class="pane-branch" style={{ flex: `0 0 ${split().ratio * 100}%` }}>
              <PaneTree node={split().children[0]} path={[...props.path, 0]} />
            </div>
            <PaneResizer dir={split().dir} path={props.path} />
            <div class="pane-branch" style={{ flex: `0 0 ${(1 - split().ratio) * 100}%` }}>
              <PaneTree node={split().children[1]} path={[...props.path, 1]} />
            </div>
          </div>
        );
      }}
    </Show>
  );
}

function PaneResizer(props: { dir: "row" | "col"; path: number[] }): JSX.Element {
  const windowId = useWindowId(); // the seam's path is relative to its own window's tree
  return (
    <div
      class={`pane-resizer pane-resizer-${props.dir}`}
      classList={{ "pane-seam-selected": samePaneTarget(paneSel(), { kind: "seam", path: props.path }) }}
      data-pane-seam-path={props.path.join(".")}
      data-pane-seam-dir={props.dir}
      onPointerDown={(e) => {
        e.preventDefault();
        const box = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
        const win = windowOf(e); // the drag continues in the seam's own window
        const onMove = (ev: PointerEvent) => {
          const raw =
            props.dir === "row"
              ? (ev.clientX - box.left) / Math.max(1, box.width)
              : (ev.clientY - box.top) / Math.max(1, box.height);
          setSplitRatio(props.path, raw, windowId);
        };
        const onUp = () => {
          win.removeEventListener("pointermove", onMove);
          win.removeEventListener("pointerup", onUp);
        };
        win.addEventListener("pointermove", onMove);
        win.addEventListener("pointerup", onUp);
      }}
    />
  );
}

// Highlight for a selected pane-edge SEGMENT: lives inside the owning pane so
// it spans exactly that pane's side (splitting it splits only this pane).
function PaneEdgeSegHighlight(props: { paneId: string }): JSX.Element {
  const side = () => {
    const t = paneSel();
    return t?.kind === "pane-edge" && t.paneId === props.paneId ? t.side : null;
  };
  return <Show when={side()}>{(s) => <div class={`pane-edge-seg pane-edge-seg-${s()}`} />}</Show>;
}

function PaneTabSplitPreview(props: { paneId: string }): JSX.Element {
  const side = () => tabSplitPreviewSideForPane(props.paneId);
  return (
    <Show when={side()}>
      {(s) => <div class={`pane-tab-split-preview pane-tab-split-preview-${s()}`} />}
    </Show>
  );
}

function PaneContent(props: { router: PaneRouter }): JSX.Element {
  return (
    <Switch fallback={<PageView />}>
      <Match when={props.router.route().kind === "query"}>
        <QueryWorkspace route={props.router.route() as QueryRoute} router={props.router} focusSource={focusedPaneId() === props.router.paneId} />
      </Match>
      <Match when={props.router.route().kind === "conflicts"}>
        <ConflictOverview router={props.router} />
      </Match>
    </Switch>
  );
}

/** A pane's `.main-content` scroller and its page column.
 *  Contract: `natural-content-overflow` is set exactly while the column's
 *  natural height exceeds the scroller's, re-measured on either one resizing.
 *  The end-of-page slack keys off that (app.css), so long pages keep 40% tail
 *  room through read/edit transitions and fitting panes never scroll (GH #369,
 *  #390). `identifyPane: false` omits `data-pane-id` (the multi-pane leaf
 *  carries it on its wrapper). */
function PaneScroller(props: {
  paneId: string;
  router: PaneRouter;
  class?: string;
  identifyPane?: boolean;
  children: JSX.Element;
}): JSX.Element {
  let scroller!: HTMLElement;
  let inner!: HTMLDivElement;
  const [naturalOverflow, setNaturalOverflow] = createSignal(false);
  const measure = () => {
    if (!scroller?.isConnected || !inner?.isConnected) return;
    setNaturalOverflow(inner.scrollHeight > scroller.clientHeight + 1);
  };
  onMount(() => {
    measure();
    const cancelFrame = requestFrame(scroller, measure);
    const observer = newResizeObserver(scroller, measure);
    if (!observer) {
      onCleanup(cancelFrame);
      return;
    }
    observer.observe(scroller);
    observer.observe(inner);
    onCleanup(() => {
      cancelFrame();
      observer.disconnect();
    });
  });
  return (
    <main
      class={`main-content${props.class ? ` ${props.class}` : ""}`}
      classList={{ "natural-content-overflow": naturalOverflow() }}
      tabindex="-1"
      data-pane-id={props.identifyPane === false ? undefined : props.paneId}
      ref={(el) => {
        scroller = el;
        props.router.setScrollerElement(el);
      }}
    >
      <div class="main-content-inner" ref={inner}>{props.children}</div>
    </main>
  );
}

function PaneRouteBody(props: { paneId: string; router: PaneRouter; scrollerClass?: string }): JSX.Element {
  const route = () => props.router.route();
  createEffect(() => {
    if (route().kind === "pdf" || route().kind === "invalid") props.router.setScrollerElement(null);
  });
  return (
    <Show when={route().kind === "pdf" ? route() as PdfRoute : null} fallback={
      <Show when={route().kind === "invalid" ? route() as InvalidRoute : null} fallback={
        <PaneScroller paneId={props.paneId} router={props.router} class={props.scrollerClass}
          identifyPane={!props.scrollerClass}>
          <FailureBoundary region="This page">
            <PaneContent router={props.router} />
          </FailureBoundary>
        </PaneScroller>
      }>
        {(invalid) => <div class="pane-route-error" role="alert">
          <h2>{invalid().title}</h2>
          <p>{invalid().message}</p>
          <button type="button" onClick={() => { void props.router.closeTab(props.router.activeId()); }}>Close tab</button>
        </div>}
      </Show>
    }>
      {(pdf) => <div class="pdf-pane pdf-route-pane" classList={{ "pdf-pane-mobile": isMobilePlatform }}
        data-pane-id={props.scrollerClass ? undefined : props.paneId} data-pdf-view-id={pdf().viewId}>
        <FailureBoundary region="This PDF"><Suspense fallback={<div class="pdf-loading" />}>
          <KeyedPdfViewer route={() => props.router.route() as PdfRoute} owner={currentPdfOwnership}
            focused={() => focusedPaneId() === props.paneId}
            onClose={() => { void props.router.closePdf(); }}
            onOpenNotes={(block) => {
              const current = props.router.route();
              if (current.kind === "pdf") openPdfNotes(props.paneId, hlsPageName(current.filename), block);
            }}
            onViewState={(state) => props.router.updateActivePdfViewState(state)} />
        </Suspense></FailureBoundary>
      </div>}
    </Show>
  );
}

function PaneLeaf(props: { paneId: string }): JSX.Element {
  const router = paneRouter(props.paneId);
  const windowId = useWindowId();
  const multi = () => layoutHasMultiplePanes(layoutRoot(windowId));
  // STATIC per pane: context provider values freeze at mount, so the surface
  // must not depend on the pane's current route. Page.tsx's endEditForSurface
  // key uses the same mapping.
  const surface = () => (props.paneId === "main" ? "main" : `pane:${props.paneId}`);
  return (
    <PaneContext.Provider value={{ paneId: props.paneId, router }}>
      <SurfaceContext.Provider value={surface()}>
        <Show
          when={multi()}
          fallback={
            // Non-scrolling relative shell holds the pane-select overlays; the
            // scroller is the inner <main>. Mirrors the multi-pane .pane-leaf —
            // without it, the pane-edge highlight lived INSIDE the scroller and
            // scrolled off-screen on a tall page, so arrows in pane-select mode
            // looked like they did nothing on a solo pane (Martin's report).
            <div
              class="main-content-shell"
              classList={{
                "pane-selected":
                  samePaneTarget(paneSel(), { kind: "pane", paneId: props.paneId }) ||
                  tabDropHighlightsPane(props.paneId),
              }}
            >
              <PaneTabSplitPreview paneId={props.paneId} />
              <PaneEdgeSegHighlight paneId={props.paneId} />
              <PaneRouteBody paneId={props.paneId} router={router} />
            </div>
          }
        >
          <div
            class="pane-leaf"
            classList={{
              "pane-focused": focusedPaneId() === props.paneId,
              "pane-selected":
                samePaneTarget(paneSel(), { kind: "pane", paneId: props.paneId }) ||
                tabDropHighlightsPane(props.paneId),
            }}
            data-pane-id={props.paneId}
          >
            <PaneTabSplitPreview paneId={props.paneId} />
            <PaneEdgeSegHighlight paneId={props.paneId} />
            <FailureBoundary region="The tabs"><TabBar
              router={router}
              dragRegion={false}
              paneStrip
              focused={focusedPaneId() === props.paneId}
            /></FailureBoundary>
            <PaneRouteBody paneId={props.paneId} router={router} scrollerClass="pane-main-content" />
          </div>
        </Show>
      </SurfaceContext.Provider>
    </PaneContext.Provider>
  );
}

// Pane-select is a MODE entered/exited by the same key (Esc at the top of the
// ladder), so without a persistent indicator "press Esc a few times" leaves the
// user unsure whether arrows will do anything (Martin hit exactly this). The
// pill is that indicator, and doubles as in-situ docs for the seam/edge tricks.
export function PaneSelectHint(): JSX.Element {
  const kind = () => paneSel()?.kind ?? null;
  const here = paneSelectIsHere();
  return (
    <Show when={here() && paneSel()}>
      <div class="pane-select-hint">
        <span class="pane-select-hint-title">Pane select</span>
        <Show
          when={kind() !== "pane"}
          fallback={
            <span class="pane-select-hint-body">
              <span>
                <kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> move (onto seams &amp; edges) · <kbd>Enter</kbd> enter
                pane · <kbd>Del</kbd> close pane
              </span>
              <span>
                <kbd>Ctrl+K</kbd> open a page in this pane · <kbd>Esc</kbd> exit
              </span>
            </span>
          }
        >
          <span class="pane-select-hint-body">
            <span>
              <kbd>Enter</kbd>{" "}
              <Show when={kind() === "edge"} fallback={<Show when={kind() === "pane-edge"} fallback={<span>split here (mirrors the pane)</span>}><span>split <span class="pane-select-hint-em">this pane</span></span></Show>}>
                <span>split the <span class="pane-select-hint-em">whole window</span></span>
              </Show>{" "}
              · <span class="pane-select-hint-em">type a page name</span> (or <kbd>Ctrl+K</kbd>) to open it in the new
              split
            </span>
            <span>
              <Show when={kind() === "pane-edge"}>
                <span>press outward again to widen the split · </span>
              </Show>
              <kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> move · <kbd>Esc</kbd> exit
            </span>
          </span>
        </Show>
      </div>
    </Show>
  );
}

/** Pane select acts on the window the user is in (currentLayoutWindowId); its
 * hint and edge tint render only in that window, never in another one too. */
function paneSelectIsHere(): () => boolean {
  const windowId = useWindowId();
  return () => currentLayoutWindowId() === windowId;
}

export function PaneEdgeHighlights(): JSX.Element {
  const here = paneSelectIsHere();
  const edge = () => {
    if (!here()) return null;
    const target = paneSel();
    return target?.kind === "edge" ? target.side : null;
  };
  return (
    <Show when={edge()}>
      {(side) => (
        <>
          {/* A global edge can sit exactly where a pane-edge segment was (a
              full-height column's side): tint EVERYTHING so "this splits the
              whole window" is visually distinct from "this splits one pane". */}
          <div class="pane-edge-global-tint" />
          <div class={`pane-edge-highlight pane-edge-highlight-${side()}`} />
        </>
      )}
    </Show>
  );
}

/** Read the platform and install click delegation on iOS/Android while owner
 * is current; desktop or retired ownership returns inert cleanup. Clicks on
 * http, https and mailto anchors are intercepted and sent to the native
 * external opener; any other explicit scheme only loses its default navigation. The opener is fire-and-forget; its failure does not reject
 * installation. Platform-read failure rejects. Click work follows DOM
 * ancestor depth; cleanup removes the listener. */
export async function installMobileExternalLinkHandler(owner: Owner = ownedWhen()): Promise<() => void> {
  const platform = await readOwned(owner, backend().appPlatform());
  if (platform.kind === "stale" || platform.value === "desktop") return () => {};

  const onClick = (e: MouseEvent) => {
    const target = e.target;
    const el = isElementNode(target) ? target : isNodeValue(target) ? target.parentElement : null;
    const a = el?.closest?.("a[href]") as HTMLAnchorElement | null;
    const href = a?.getAttribute("href")?.trim() ?? "";
    const scheme = a ? /^([a-z][a-z0-9+.-]*):/i.exec(href)?.[1]?.toLowerCase() : undefined;
    if (!a || !scheme) return; // graph-internal relative/hash navigation
    // I-22 (master b61bb9d25303): an anchor in shared or imported content with
    // any other explicit scheme (intent:, javascript:, tel:, …) must not
    // navigate the WebView; its own handler (e.g. a file: asset link) still runs.
    e.preventDefault();
    if (scheme !== "http" && scheme !== "https" && scheme !== "mailto") return;
    e.stopPropagation();
    void backend().openExternal(a.href);
  };

  // Mobile only, where main is the sole window.
  mainWindow.document.addEventListener("click", onClick, true);
  return () => mainWindow.document.removeEventListener("click", onClick, true);
}

/** Install the graph window's capture receiver. Each request is deduplicated
 * by id (100 completed ids retained); writes use the document capture door.
 * Work is O(captured blocks + destination page). Transport setup failures reject;
 * Requests must carry the native capture-show binding generation; stale or
 * missing generations acknowledge false. The surface owner covers registration
 * and callbacks; writes capture graphOwner before starting. Save failure
 * acknowledges false and preserves the sender's scratch. Dispose
 * the returned listener when the app surface retires. */
export async function installQuickCaptureReceiver(live: WriteOwner = ownedWhen(() => true)): Promise<() => void> {
  const owner = ownedWhen(live);
  const inFlight = new Map<string, Promise<boolean>>();
  const completed = new Map<string, boolean>();
  const completedOrder: string[] = [];
  const rememberCompleted = (id: string, ok: boolean) => {
    completed.set(id, ok);
    completedOrder.push(id);
    while (completedOrder.length > 100) {
      const old = completedOrder.shift();
      if (old) completed.delete(old);
    }
  };
  const { emitTo, listen } = await import("@tauri-apps/api/event");
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const windowLabel = getCurrentWindow().label;
  const ack = (id: string | undefined, ok: boolean) => {
    if (id) void emitTo("capture", "quick-capture-ack", { id, ok } satisfies QuickCaptureAck);
  };
  if (!owner()) return () => {};
  const registration = await readOwnedResource(owner, listen<QuickCaptureRequest & { bindingGeneration: number }>("quick-capture", async (e) => {
    // WebKitGTK currently exposes targeted Tauri events to every graph
    // listener in this process. Treat the payload label as the authority so
    // only the selected graph can ever perform the write.
    if (!owner() || e.payload?.target !== windowLabel) return;
    const id = e.payload?.id;
    if (id && completed.has(id)) {
      ack(id, completed.get(id) ?? false);
      return;
    }
    const existing = id ? inFlight.get(id) : undefined;
    if (existing) {
      ack(id, await existing);
      return;
    }
    if (!Number.isSafeInteger(e.payload.bindingGeneration) || e.payload.bindingGeneration <= 0
        || e.payload.bindingGeneration !== backend().graphBindingGeneration()) {
      ack(id, false);
      return;
    }
    const saveOwner = bindingOwner(owner);
    const text = e.payload?.text ?? "";
    if (!text.trim()) {
      ack(id, false);
      return;
    }
    // A title routes the capture to a NEW (or existing) page; empty → today.
    const title = (e.payload?.title ?? "").trim();
    const save = async () => {
      let ok = false;
      try {
        const result = await writeOwned(saveOwner, title ? captureToPage(title, text) : appendToTodayJournal(text));
        ok = result.kind === "current" && result.value;
      } catch {
        ok = false;
      }
      if (saveOwner()) pushToast(
        ok
          ? title
            ? `Captured to “${title}”`
            : "Captured to today's journal"
          : "Capture couldn't be saved — its text is kept in the capture window",
        ok ? "info" : "error"
      );
      return ok;
    };
    const promise = save();
    if (id) inFlight.set(id, promise);
    const ok = await promise;
    if (id) {
      inFlight.delete(id);
      rememberCompleted(id, ok);
    }
    ack(id, ok);
  }), (unlisten) => unlisten());
  return registration.kind === "current" ? registration.value : () => {};
}

export function App(): JSX.Element {
  installDraftStore();
  // Every graph window mounts App and owns its own save engine. Split panes
  // share it; the capture mini-window owns only an unsaved scratch page.
  onMount(() => onCleanup(installBackgroundFlush({
    endEdit: () => endEdit("graph-switch"),
    flushAll,
    closeInFlight: safeClose.inFlight,
  })));
  // GH #426: on mobile an OS reap of a hidden app is not an unclean exit.
  onMount(() => onCleanup(installSessionActivity({
    isMobile: isMobilePlatform,
    setActive: (active) => void recordSessionActive(active),
  })));
  let openCalendarJump = () => {};
  const topbarActions = {
    calendar: () => openCalendarJump(),
    journals: () => openJournals(),
    theme: () => toggleTheme(),
    rightSidebar: (trigger?: HTMLElement | null) => toggleRightSidebar(trigger),
    back: () => goBack(),
    forward: () => goForward(),
  };
  // Startup debug trace (TINE_DEBUG=1 / --debug): forward UI milestones + errors
  // into the backend log so a remote "bad startup" is diagnosable in one file.
  onMount(() => void initDebug());

  // ONE Back ladder (src/appBack.ts) serves both Back gestures: Android's
  // native SafeBack listener (below) and the iOS left-edge swipe (installed
  // after it).  A drawer/transient is never represented by synthetic history;
  // route history remains the fallback.
  const backDeps = {
    dismissTransient: () => dismissTopTransient("back"),
    dismissDrawer: () => dismissMobileDrawer("back"),
    restoreDrawerFocus: () => restoreDrawerFocus("back"),
    historyBack: () => {
      if (!canGoBack()) return false;
      goBack();
      return true;
    },
    closeRoot: () => { void closeAndroidRootSafely(); },
  };
  onMount(() => {
    if (!isTauri()) return;
    const uninstall = installAndroidBackHandler({
      platform: () => backend().appPlatform(),
      // The permanent native owner (MainActivity's OnBackPressedCallback +
      // SafeBackPlugin) forwards Back here.  AppPlugin's own listener is NOT
      // used: with none registered Tauri falls back to WebView.goBack()/finish,
      // which bypasses the ladder (master 61a663291 and successors).
      subscribe: async (handler) => {
        const { addPluginListener } = await import("@tauri-apps/api/core");
        return addPluginListener("safe-back", "android-safe-back", handler);
      },
      ...backDeps,
      // No JS listener: the native owner stays registered and blocks Back
      // (with a throttled notice) rather than letting the WebView navigate.
      setupFailed: () => console.warn("Android SafeBack listener unavailable; native owner remains blocking"),
    });
    onCleanup(uninstall);
  });

  // iOS Back and the left-drawer pull share ONE left-edge recognizer
  // (src/edgeSwipe.ts); the ladder it runs on commit is the same `backDeps`.
  onMount(() => {
    if (!isTauri()) return;
    let disposed = false;
    let uninstall: () => void = () => {};
    void (async () => {
      let native: Owned<"android" | "ios" | "desktop">;
      try {
        native = await readOwned(ownedWhen(() => !disposed), backend().appPlatform());
      } catch {
        console.warn("edge swipe: platform unavailable, left-edge gestures stay off");
        return;
      }
      if (native.kind === "stale") return;
      // touchGesturePlatform() is the real mobile platform, or - only under the
      // TINE_E2E_TOUCH_GESTURES harness hook - the platform the journey asks for.
      const platform = touchGesturePlatform() ?? native.value;
      if (disposed || platform === "desktop") return;
      uninstall = installEdgeSwipe({
        platform,
        backAvailable: () => appBackAvailable({
          hasTransient: () => topTransientLayer() !== undefined,
          hasDrawer: () => activeDrawer() !== null,
          canGoBack,
        }),
        drawerOpenable: () => mobileDrawerMode() && activeDrawer() === null,
        // The edge swipe never closes the app: iOS has no root rung.
        back: () => { dispatchAppBack({ ...backDeps, closeRoot() {} }); },
        openDrawer: () => setLeftSidebarOpen(true),
        surface: () => mainWindow.document.querySelector<HTMLElement>(".app-container > .main-container"),
      });
    })();
    onCleanup(() => { disposed = true; uninstall(); });
  });

  onMount(async () => {
    let alive = true;
    let disposeLinks = () => {};
    onCleanup(() => { alive = false; disposeLinks(); });
    const owner = graphOwner(() => alive);
    const injected = (window as any).__GRAPH_PATH__ ?? "";
    let startup = "";
    try {
      const result = await readOwned(owner, backend().startupGraphPath());
      if (result.kind === "stale") return;
      startup = result.value ?? "";
    } catch {
      startup = "";
    }
    const graphPath = injected || startup || persistedGraphPath();
    dbg(`loading graph: ${graphPath || "(default/configured)"}`);
    try {
      if (!(window as any).__TINE_LINK_LAUNCH__ || injected) await loadGraphPath(graphPath);
      dbg("graph load call returned");
    } catch (e) {
      // No graph configured (fresh install), or it failed to open. Fall through to
      // the onboarding Welcome screen instead of leaving a blank app; don't toast
      // on first run (the empty/`""` path legitimately has no graph yet).
      dbg(`graph load failed: ${String(e)}`);
      if (graphPath) setStartupOpenFailure({ path: graphPath, message: String(e) });
    } finally {
      if (isPublishedExport() && window.location.hash) {
        try {
          openPublishedPermalink(await loadPublishedSnapshot(), window.location.hash, paneRouter(focusedPaneId()));
        } catch { console.error("published permalink unavailable"); }
      }
      // The load above retires `owner` (opening a graph moves the binding), so the
      // VIEW, not the graph binding, owns this completion.
      if (alive) setFirstLoadDone(true);
      if (!isPublishedExport() && alive) {
        disposeLinks = await installTineLinks(() => alive);
        if (!alive) disposeLinks();
      }
    }
  });

  createEffect(() => {
    if (!isPublishedExport() || !firstLoadDone() || !graphMeta()) return;
    const panes = layoutPaneIds(layoutRoot(MAIN_WINDOW_ID));
    const router = paneRouter(panes[0] ?? focusedPaneId(MAIN_WINDOW_ID));
    const target = publishedPermalinkForWorkspace(panes.length, router.tabs().length, router.route());
    if (target !== undefined) replacePublishedPermalink(target);
  });
  onMount(() => {
    if (!isPublishedExport()) return;
    const onHash = () => { void loadPublishedSnapshot().then((snapshot) => {
      openPublishedPermalink(snapshot, window.location.hash, paneRouter(focusedPaneId()));
    }); };
    window.addEventListener("hashchange", onHash);
    onCleanup(() => window.removeEventListener("hashchange", onHash));
  });

  // Warn (loudly) if the webview is painting on the CPU — Tine's whole pitch is
  // speed, so a silent software-rendering fallback shouldn't read as "Tine is
  // slow". Fire-and-forget; the probe is Tauri-gated and never throws.
  onMount(() => void warnIfSoftwareRendering());

  // Native startup owns the one-shot migration flag; this view owns its toast.
  onMount(async () => {
    let alive = true;
    onCleanup(() => { alive = false; });
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      if (!alive) return;
      const result = await readOwned(ownedWhen(() => alive), invoke<boolean>("take_identifier_migration_notice"));
      if (result.kind === "stale" || !result.value) return;
      pushToast(
        "Tine was renamed under the hood, so we moved your settings and backups across. A few app-level preferences (e.g. keyboard shortcuts) might need setting again — sorry about that!",
        "info",
        { sticky: true },
      );
    } catch {
      dbg("identifier migration notice unavailable");
    }
  });

  // An unwritable app-data folder was relocated for this launch (data_home.rs):
  // say where settings and backups went, stickily — a silent relocation would
  // be its own defect (I-9).
  onMount(async () => {
    let alive = true;
    onCleanup(() => { alive = false; });
    try {
      const result = await readOwned(ownedWhen(() => alive), backend().takeDataHomeFallbackNotice());
      if (result.kind === "stale" || !result.value) return;
      pushToast(
        `Tine could not write its usual application-data folder, so this session is keeping settings and backups in ${result.value} instead. Fixing the permissions on that folder restores the normal location.`,
        "warn",
        { sticky: true },
      );
    } catch {
      dbg("data-home notice unavailable");
    }
  });

  // GH #623: once per opened graph, ask whether Windows Defender is the likely
  // reason the cold load was slow. The backend answers false everywhere else.
  createEffect(on(() => graphMeta()?.root, (root) => {
    if (root) void maybeShowDefenderHint();
  }));

  // The updater owns preference loading, automatic scheduling and cancellation.
  onMount(() => onCleanup(scheduleAutomaticUpdateCheck()));

  // Re-install experimental smooth scrolling (Lenis) if it was left on. The feed
  // (`.main-content`) is mounted by now (onMount runs after first render).
  onMount(() => void initSmoothScroll());
  onMount(() => void initCopySettings());
  onMount(() => void initRefCompletionSettings());
  onMount(() => void initNavSettings());
  onMount(() => void initSettingsLayout());
  onMount(() => { void initContentWidths(); void initCodeDisplay(); });
  // Load the local-file images opt-in (Settings → Editing). Default off.
  onMount(() => void initLocalFileSettings());
  // A conflict copy appearing/vanishing on disk (watcher) refreshes the list.
  onMount(() => {
    let unsub = () => {};
    let alive = true;
    const owner = ownedWhen(() => alive);
    void readOwnedResource(owner, backend().onConflictsChanged(() => void refreshSyncConflicts("new")), (u) => u())
      .then((result) => { if (result.kind === "current") unsub = result.value; });
    onCleanup(() => { alive = false; unsub(); });
  });
  onMount(() => {
    let unsub = () => {};
    let alive = true;
    const owner = ownedWhen(() => alive);
    void readOwnedResource(owner, backend().onGraphConfigChanged(applyGraphConfigChange), (u) => u())
      .then((result) => { if (result.kind === "current") unsub = result.value; });
    onCleanup(() => { alive = false; unsub(); });
  });
  // One graph-file watcher for every pane. PageView instances render pane
  // content; they do not each own a backend subscription.
  onMount(() => {
    let unsub = () => {};
    let alive = true;
    const owner = ownedWhen(() => alive);
    void readOwnedResource(owner, backend().onGraphChanged((c) => trackGraphChangeApplication(applyGraphChange(c))), (u) => u())
      .then((result) => { if (result.kind === "current") unsub = result.value; });
    onCleanup(() => { alive = false; unsub(); });
  });
  // Family 10: checkout-sized batches, a refused OS watch, reload on focus,
  // and the "always ask" preference.
  onMount(() => {
    onCleanup(subscribeWatcherFreshness());
    onCleanup(subscribeAssetChanges());
    installReloadOnFocus();
    void initConflictPolicy();
  });
  // Load the asset-filename format template (Settings → Backups → Asset names).
  onMount(() => void initAssetSettings());
  // Load external media-editor command templates (Settings → Files; GH #38).
  onMount(() => void initMediaEditorSettings());
  // Load spellcheck prefs (toggle + languages) and apply them to the webview.
  onMount(() => void initSpellcheckSettings());
  // Load the `[[`/`#` autocomplete default-action preference (link-first vs create).
  onMount(() => void initLinkDefault());

  // Android/iOS WebViews otherwise navigate raw target=_blank links in-app.
  onMount(() => {
    let uninstall = () => {};
    let disposed = false;
    void installMobileExternalLinkHandler(ownedWhen(() => !disposed)).then((u) => {
      if (disposed) u();
      else uninstall = u;
    });
    onCleanup(() => {
      disposed = true;
      uninstall();
    });
  });

  // Persist pending edits before the window closes — the 400ms save debounce
  // would otherwise drop the last keystrokes typed right before quitting.
  // Hardened so it can NEVER wedge the window open: a re-entry guard, a timeout
  // cap on the flush, and a destroy()→close() fallback.
  onMount(() => {
    if (!isTauri()) return;
    let unlisten = () => {};
    let closeInProgress = false;
    let allowClose = false;
    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const w = getCurrentWindow();
      unlisten = await w.onCloseRequested(async (e) => {
        if (allowClose) return; // second pass (from close() below) — let it through
        e.preventDefault();
        if (closeInProgress) return;
        closeInProgress = true;
        if ((await safeClose.prepare()) !== "accepted") { closeInProgress = false; return; }
        allowClose = true;
        // The session (saved by prepare) lists the open workspace windows; close them now.
        closeAllWorkspaceWindows("quit");
        // Close only this graph window. The backend exits the process (including
        // Linux WebKit cleanup) only when this is the final graph window.
        try {
          await writeOwned(bindingOwner(), backend().closeGraphWindow());
          return;
        } catch {
          // fall through to the direct close below
        }
        try {
          await w.destroy();
        } catch {
          try { await w.close(); } // re-fires onCloseRequested; the guard lets it close
          catch {
            // The native close attempt failed. Re-arm the persistence guard as
            // well as the shared transaction before a later close request;
            // leaving allowClose=true would let that retry bypass saving.
            allowClose = false;
            safeClose.reset();
            closeInProgress = false;
          }
        }
      });
    })();
    onCleanup(() => unlisten());
  });

  // Global quick-capture: a `tine --capture` launch (bound to a DE hotkey)
  // signals the running app to pop the capture mini-window; on submit it emits a
  // `quick-capture` event that the selected graph window turns into an append to today's
  // journal. Going through the live store (not a separate file writer) keeps a
  // capture from racing a main-view edit of today's journal into a conflict.
  onMount(() => {
    if (!isTauri()) return;
    let alive = true, unlisten = () => {};
    onCleanup(() => { alive = false; unlisten(); });
    void installQuickCaptureReceiver(ownedWhen(() => alive)).then((dispose) => { if (alive) unlisten = dispose; else dispose(); })
      .catch(() => { if (alive) pushToast("Quick Capture could not connect to this graph window.", "error"); });
  });

  // Tell the quick-capture mini-window our theme. It can't read the main
  // window's localStorage (WebKitGTK doesn't share it across webviews), so it
  // requests the theme when shown and we reply; we also broadcast on every
  // change so an open capture window updates live.
  onMount(() => {
    if (!isTauri()) return;
    let unlisten = () => {};
    void (async () => {
      const { emitTo, listen } = await import("@tauri-apps/api/event");
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const windowLabel = getCurrentWindow().label;
      unlisten = await listen<{ target: string }>("capture-request-theme", (e) => {
        if (e.payload?.target !== windowLabel) return;
        void emitTo("capture", "capture-apply-theme", { theme: theme() });
      });
    })();
    onCleanup(() => unlisten());
  });

  // OS file drag-and-drop → insert dropped files as assets at the drop target.
  onMount(() => {
    if (!isTauri()) return;
    let uninstall = () => {};
    void installFileDrop().then((u) => (uninstall = u));
    onCleanup(() => uninstall());
  });
  createEffect(() => {
    const t = theme();
    if (!isTauri()) return;
    const owner = graphOwner();
    void (async () => {
      try {
        const { emitTo } = await import("@tauri-apps/api/event");
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const target = await readOwned(owner, backend().captureTarget());
        if (target.kind === "current" && target.value === getCurrentWindow().label) {
          await emitTo("capture", "capture-apply-theme", { theme: t });
        }
      } catch {
        // No graph is bound yet (Welcome) or capture is unavailable.
      }
    })();
  });

  // The page index refetches `page_inventory` on graph bind, after edits settle
  // (dataRev: an alias:: edit must not leave navigation on the old page), and on
  // create/delete/rename (pageInventoryRev); one IPC per trigger tick.
  installPageIndex();

  // (Re)install keybindings whenever config or the user's local overrides change
  // (precedence: defaults < config.edn :shortcuts < Settings overrides). We also
  // mirror the merged map to the quick-capture window so a remapped
  // editor/quick-capture-file (or any editor shortcut) is honored there too — it
  // can't read this window's localStorage overrides on its own.
  let latestShortcuts: Record<string, string> = {};
  const captureBroadcastScope = {};
  const broadcastShortcuts = () => {
    if (!isTauri()) return;
    const owner = latestOwner(captureBroadcastScope, "shortcuts", graphOwner());
    void (async () => {
      try {
        const { emitTo } = await import("@tauri-apps/api/event");
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const target = await readOwned(owner, backend().captureTarget());
        if (target.kind === "current" && target.value === getCurrentWindow().label) {
          await emitTo("capture", "capture-apply-shortcuts", latestShortcuts);
        }
      } catch {
        // No graph is bound yet (Welcome) or capture is unavailable.
      }
    })();
  };
  createEffect(() => {
    const cfg = graphMeta()?.shortcuts ?? {};
    const merged = { ...cfg, ...shortcutOverrides() };
    latestShortcuts = merged;
    const dispose = installKeybindings(merged);
    broadcastShortcuts();
    onCleanup(dispose);
  });
  onMount(() => {
    if (!isTauri()) return;
    let unlisten = () => {};
    void (async () => {
      const { emitTo, listen } = await import("@tauri-apps/api/event");
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const windowLabel = getCurrentWindow().label;
      unlisten = await listen<{ target: string }>("capture-request-shortcuts", (e) => {
        if (e.payload?.target !== windowLabel) return;
        void emitTo("capture", "capture-apply-shortcuts", latestShortcuts);
      });
    })();
    onCleanup(() => unlisten());
  });

  // Mouse-drag block selection: a drag that crosses a block boundary switches
  // from in-textarea text selection to whole-block selection (OG behavior).
  onMount(() => onCleanup(installBlockSelectionDrag()));

  // Interface zoom (Ctrl +/-/0): restore the saved level, track which pane is
  // focused, and own the zoom keys when the notes pane is active (the PDF pane
  // keeps them for its own zoom).
  onMount(() => {
    applyZoom();
    onCleanup(installPaneTracker());
    onCleanup(installInterfaceZoomKeys());
    onCleanup(installInterfaceZoomWheel());
  });

  // Frameless window: the toolbar doubles as the title bar (decorations are off),
  // so track maximized state to drive our custom max/restore glyph + resize grips.
  // Also apply the persisted native-frame preference (Linux/Windows; macOS uses its
  // build-time Overlay title bar — see nativeChrome.ts).
  onMount(() => {
    if (!isTauri()) return;
    void initNativeChrome();
    onCleanup(installWindowChrome());
  });

  return (
    <div
      class="app-container"
      data-mobile-drawer-mode={mobileDrawerMode() ? "true" : "false"}
      data-active-drawer={activeDrawer() ?? ""}
      classList={{
        "sidebar-collapsed": !sidebarOpen(),
        "wide-mode": wideMode(),
        "document-mode": documentMode(),
        "focus-mode": focusMode(),
        // macOS draws a transparent Overlay title bar over our content (rounded
        // corners + traffic lights); reserve the top-left so the lights don't sit
        // on the sidebar header / sidebar-toggle button. See nativeChrome.ts + app.css.
        "mac-overlay": isMac && isTauri(),
        // When on, the whole reading surface fades to a calm wash; the block
        // you're editing pops back to full opacity (the typewriter "spotlight the
        // line"). Applied whenever dim is on — not only while editing — so that
        // toggling dim (t b) or entering focus (t f) is visible immediately.
        "dim-mode": dimInactiveBlocks(),
      }}
    >
      <Show when={parserFailed()}>
        <DrawerBackground class="parser-error-banner" blockedBy="any" role="alert">
          The block renderer failed to load — text is shown unformatted. Please reload Tine;
          if this persists, report it.
        </DrawerBackground>
      </Show>
      <Show when={graphTransitioning()}>
        <DrawerBackground class="graph-transition-shield" blockedBy="any" role="status" ariaLive="polite">
          Finishing graph operation…
        </DrawerBackground>
      </Show>
      <Show when={sidebarOpen()}>
        <MobileDrawerPanel
          side="left"
          label="Navigation sidebar"
          class="left-sidebar"
          style={{
            flex: `0 0 ${sidebarWidth()}px`,
            width: `${sidebarWidth()}px`,
            "--mobile-drawer-width": `${sidebarWidth()}px`,
          }}
        >
          <div class="left-sidebar-scroll">
            <div class="sidebar-header workspace-sidebar-header" data-workspace-switcher-sidebar>
              <Show when={!isPublishedExport()}><FailureBoundary region="The workspace switcher"><WorkspaceSwitcher /></FailureBoundary></Show>
            </div>
            <Show when={mobileDrawerMode()}>
              <button class="mobile-drawer-close" type="button" aria-label="Close navigation sidebar" onClick={() => dismissDrawerAndRestore("explicit")}>Close</button>
            </Show>
            <FailureBoundary region="The sidebar">
              <Sidebar onActiveNavigationComplete={completeActiveLeftNavigation} />
            </FailureBoundary>
          </div>
          <div
            class="sidebar-resizer"
            onMouseDown={(e) => {
              e.preventDefault();
              const onMove = (ev: MouseEvent) =>
                resizeSidebar("left", ev.clientX);
              const onUp = () => {
                mainWindow.removeEventListener("mousemove", onMove);
                mainWindow.removeEventListener("mouseup", onUp);
                commitSidebarWidth("left");
              };
              // The left sidebar exists only in the main window.
              mainWindow.addEventListener("mousemove", onMove);
              mainWindow.addEventListener("mouseup", onUp);
            }}
          />
        </MobileDrawerPanel>
      </Show>
      <DrawerBackground class="main-container" blockedBy="left">
        {/* In focus mode the topbar is hidden; this thin strip at the very top
            reveals it on hover (CSS adjacency), so controls are reachable. */}
        <DrawerBackground blockedBy="right">
          <Show when={focusMode()}>
            <div class="topbar-hover-zone" />
          </Show>
        {/* The toolbar doubles as the title bar: data-tauri-drag-region lets the
            user drag the window by its empty areas (buttons/tabs, being children
            without the attribute, still click normally; double-click maximizes). */}
        <header class="topbar" data-tauri-drag-region>
          <div class="topbar-left">
            <button
              class="icon-btn"
              title="Toggle sidebar (t l)"
              onClick={(event) => toggleSidebar(event.currentTarget)}
            >
              <svg viewBox="0 0 24 24" class="nav-icon">
                <rect x="3" y="4" width="18" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.7" />
                <line x1="9" y1="4" x2="9" y2="20" stroke="currentColor" stroke-width="1.7" />
              </svg>
            </button>
            <button
              class="icon-btn"
              title="Search (Ctrl+K)"
              aria-label="Search"
              data-search-trigger
              data-pane-focus-neutral
              onClick={() => openSwitcher()}
            >
              <svg viewBox="0 0 24 24" class="nav-icon">
                <circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" stroke-width="1.7" />
                <line x1="16.5" y1="16.5" x2="21" y2="21" stroke="currentColor" stroke-width="1.7" />
              </svg>
            </button>
            <button
              class="icon-btn topbar-navigation-action"
              title="Go back"
              data-pane-focus-neutral
              disabled={!canGoBack()}
              onClick={topbarActions.back}
            >
              <svg viewBox="0 0 24 24" class="nav-icon">
                <path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
            <button
              class="icon-btn topbar-navigation-action"
              title="Go forward"
              data-pane-focus-neutral
              disabled={!canGoForward()}
              onClick={topbarActions.forward}
            >
              <svg viewBox="0 0 24 24" class="nav-icon">
                <path d="M9 5l7 7-7 7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
          </div>
          {/* A collapsed sidebar has no mounted sidebar header. Keep a compact
              one-tap workspace path in the toolbar without putting its full
              non-shrinking label back in this no-wrap row. */}
          <Show when={!sidebarOpen() && !isPublishedExport()}>
            <FailureBoundary region="The workspace switcher"><WorkspaceSwitcher compact /></FailureBoundary>
          </Show>
          {/* The tab strip is a desktop feature; on a phone it only crowds the
              single-row toolbar (and its pill clips). Hide it there, keeping a
              flex spacer so the right-side icons stay pinned to the edge. */}
          <Show when={!isMobilePlatform && !layoutHasMultiplePanes(layoutRoot(MAIN_WINDOW_ID))} fallback={<div class="topbar-spacer" data-tauri-drag-region />}>
            {/* Keyed on the SOLE pane's id: after closing panes the survivor
                need not be "main", and TabBar freezes its router at mount. */}
            <Show when={firstPaneId(layoutRoot(MAIN_WINDOW_ID)) ?? "main"} keyed>
              {(soloId) => <FailureBoundary region="The tabs"><TabBar router={paneRouter(soloId)} /></FailureBoundary>}
            </Show>
          </Show>
          <div class="topbar-right">
            <FailureBoundary region="The calendar"><CalendarJump triggerClass="topbar-optional-action" onOpenReady={(open) => { openCalendarJump = open; }} /></FailureBoundary>
            <button class="icon-btn topbar-optional-action" title="Journals" data-pane-focus-neutral onClick={topbarActions.journals}>
              <svg viewBox="0 0 24 24" class="nav-icon">
                <path d="M4 5h11a2 2 0 0 1 2 2v12H6a2 2 0 0 1-2-2V5z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" />
                <line x1="8" y1="9" x2="14" y2="9" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
                <line x1="8" y1="12.5" x2="14" y2="12.5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
                <path d="M17 5h3v14a2 2 0 0 1-2 2 1 1 0 0 1-1-1V5z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" />
              </svg>
            </button>
            <button class="icon-btn topbar-optional-action" title="Toggle theme (t t)" onClick={topbarActions.theme}>
              <Show
                when={theme() === "light"}
                fallback={
                  <svg viewBox="0 0 24 24" class="nav-icon">
                    <path
                      d="M21 12.8A9 9 0 1111.2 3 7 7 0 0021 12.8z"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.7"
                    />
                  </svg>
                }
              >
                <svg viewBox="0 0 24 24" class="nav-icon">
                  <circle cx="12" cy="12" r="5" fill="none" stroke="currentColor" stroke-width="1.6" />
                  <line x1="12" y1="2" x2="12" y2="5" stroke="currentColor" stroke-width="1.6" />
                  <line x1="12" y1="19" x2="12" y2="22" stroke="currentColor" stroke-width="1.6" />
                  <line x1="2" y1="12" x2="5" y2="12" stroke="currentColor" stroke-width="1.6" />
                  <line x1="19" y1="12" x2="22" y2="12" stroke="currentColor" stroke-width="1.6" />
                </svg>
              </Show>
            </button>
            <button
              class="icon-btn topbar-sidebar-action"
              classList={{ active: rightSidebarOpen() }}
              title="Toggle right sidebar (t r)"
              onClick={(event) => topbarActions.rightSidebar(event.currentTarget)}
            >
              <svg viewBox="0 0 24 24" class="nav-icon">
                <rect x="3" y="4" width="18" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.7" />
                <line x1="15" y1="4" x2="15" y2="20" stroke="currentColor" stroke-width="1.7" />
              </svg>
            </button>
            <FailureBoundary region="The toolbar menu"><TopbarOverflowMenu
              onCalendar={topbarActions.calendar}
              onJournals={topbarActions.journals}
              onToggleTheme={topbarActions.theme}
              onToggleRightSidebar={topbarActions.rightSidebar}
              onBack={topbarActions.back}
              onForward={topbarActions.forward}
              canGoBack={canGoBack}
              canGoForward={canGoForward}
            /></FailureBoundary>
            {/* Settings sits apart at the far right (separated by a divider) so
                it reads as app-level config, not another content control. */}
            <Show when={!isPublishedExport()}><span class="topbar-sep" />
            <button class="icon-btn" title="Settings (t s)" onClick={() => openSettings()}>
              <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
                <path
                  fill="currentColor"
                  d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.488.488 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 00-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 00-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.07.62-.07.94s.02.64.07.94l-2.03 1.58a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 00-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"
                />
              </svg>
            </button></Show>
            {/* Frameless-window controls live at the very right, where the native
                title bar's buttons used to be. Hidden when the OS draws its own
                (macOS Overlay always; Linux/Windows when the native-frame toggle
                is on). */}
            <Show when={isTauri() && !osDrawsWindowControls()}>
              <span class="topbar-sep" />
              <FailureBoundary region="Window controls"><WindowControls /></FailureBoundary>
            </Show>
          </div>
        </header>
        <FailureBoundary region="The conflict notice"><ConflictBar /></FailureBoundary>
        <Show when={refreshingFromDisk()}>
          <div class="focus-refresh-status" role="status" aria-live="polite">Refreshing changes from disk…</div>
        </Show>
        <FailureBoundary region="Find in page"><InPageFind /></FailureBoundary>
        </DrawerBackground>
        {/* Everything below the topbar lives in this row, so the topbar (and its
            window controls at the far right) spans the full window width and the
            right sidebar / PDF pane sit UNDER it — not beside the close button. */}
        <div class="content-row">
          <DrawerBackground class="drawer-workspace" blockedBy="right">
          <PaneEdgeHighlights />
          <PaneSelectHint />
          <PaneTree node={visibleLayoutNode(MAIN_WINDOW_ID)} path={[]} />
          </DrawerBackground>
          <FailureBoundary region="The reference sidebar"><RightSidebar /></FailureBoundary>
        </div>
      </DrawerBackground>
      <MobileDrawerController />
      <DrawerBackground class="drawer-floating-background" blockedBy="any">
        <Show when={focusMode()}>
          <button class="focus-exit" title="Exit focus (Esc)" onClick={() => void exitFocusMode()}>
            <svg viewBox="0 0 24 24" class="nav-icon">
              <path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
            </svg>
          </button>
        </Show>
        <Show when={isTauri() && !osDrawsWindowControls() && !maximized()}>
          <ResizeGrips />
        </Show>
      </DrawerBackground>
      <WindowOverlays
        windowId={MAIN_WINDOW_ID}
        deepLink={<FailureBoundary region="The graph chooser"><DeepLinkGraphChoice /></FailureBoundary>}
        keyboardToolbar={<DrawerBackground class="drawer-floating-background" blockedBy="any">
          <FailureBoundary region="The keyboard toolbar"><MobileKeyboardToolbar /></FailureBoundary>
        </DrawerBackground>}
        recovery={<FailureBoundary region="Unsaved recovery"><UnsavedRecovery /></FailureBoundary>}
        // First-run onboarding covers the (empty) app when no graph is configured;
        // it renders before Toasts so a "couldn't create graph" toast shows on top.
        welcome={<FailureBoundary region="Welcome"><WelcomeLayer
          mandatory={(globalThis as any).__FORCE_WELCOME__ === true || (firstLoadDone() && !graphMeta())}
          optionalOpen={welcomeOpen()}
          onClose={closeWelcome}
        /></FailureBoundary>}
      />
    </div>
  );
}
