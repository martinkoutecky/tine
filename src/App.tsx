import { Show, Suspense, createEffect, lazy, on, onCleanup, onMount, type JSX } from "solid-js";
import { Sidebar } from "./components/Sidebar";
import { PageView, reloadJournalsFeedFromStart, toLoadablePage, type JournalsFeedOwner } from "./components/Page";
import { QueryWorkspace } from "./components/QueryWorkspace";
import { QuickSwitcher } from "./components/QuickSwitcher";
// pdf.js (~hundreds of KB) is heavy and most sessions never open a PDF — load
// the viewer only when one is opened.
const KeyedPdfViewer = lazy(() =>
  import("./components/PdfViewer").then((m) => ({ default: m.KeyedPdfViewer }))
);
import { TabBar, tabDropHighlightsPane, tabSplitPreviewSideForPane } from "./components/TabBar";
import { ContextMenu } from "./components/ContextMenu";
import { Toasts, Lightbox } from "./components/Toasts";
import { AudioOverlay } from "./components/AudioOverlay";
import { CalendarJump } from "./components/CalendarJump";
import { ConflictBar } from "./components/ConflictBar";
import { RightSidebar } from "./components/RightSidebar";
import { Settings } from "./components/Settings";
import { HelpPopup } from "./components/HelpShortcuts";
import { DatePicker } from "./components/DatePicker";
import { FormulaEditor } from "./components/FormulaEditor";
import { MobileKeyboardToolbar } from "./components/MobileKeyboardToolbar";
import { PageProps } from "./components/PageProps";
import { ExportModal } from "./components/ExportModal";
import { PdfExportDialog } from "./components/PdfExportDialog";
import { InPageFind } from "./components/InPageFind";
import { installKeybindings } from "./keybindings";
import { installFileDrop } from "./filedrop";
import { installBlockSelectionDrag } from "./blockDrag";
import { loadGraphPath, persistedGraphPath, refreshAliases } from "./graph";
import { checkForUpdate } from "./update";
import { Welcome } from "./components/Welcome";
import { goBack, goForward, canGoBack, canGoForward, flushSession, openJournals, sameRoute, type PaneRouter, type QueryRoute } from "./router";
import {
  theme,
  toggleTheme,
  sidebarOpen,
  toggleSidebar,
  rightSidebarOpen,
  toggleRightSidebar,
  openSwitcher,
  pdfTarget,
  pdfPaneWidth,
  setPdfPaneWidth,
  persistPdfPaneWidth,
  sidebarWidth,
  setSidebarWidth,
  persistSidebarWidth,
  graphMeta,
  firstLoadDone,
  setFirstLoadDone,
  openSettings,
  welcomeOpen,
  closeWelcome,
  shortcutOverrides,
  wideMode,
  documentMode,
  focusMode,
  dimInactiveBlocks,
  exitFocusMode,
  dataRev,
  bumpDataRev,
  installPaneTracker,
  markConflict,
  pushToast,
  refreshSyncConflicts,
  graphEpoch,
  graphTransitioning,
  setGraphTransitioning,
} from "./ui";
import { applyZoom, installInterfaceZoomKeys, installInterfaceZoomWheel } from "./zoom";
import {
  doc,
  flushAll,
  appendToTodayJournal,
  captureToPage,
  pageByName,
  reloadDisposition,
  reloadPage,
  restoreTodayJournalInFeed,
} from "./store";
import type { QuickCaptureAck, QuickCaptureRequest } from "./quickCaptureAck";
import { backend, isTauri, type GraphChange } from "./backend";
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
import { initDebug, dbg } from "./debug";
import { WindowControls, ResizeGrips, installWindowChrome, maximized } from "./components/WindowChrome";
import { initNativeChrome, isMac, isMobilePlatform, osDrawsWindowControls } from "./nativeChrome";
import {
  PaneContext,
  closePane,
  firstPaneId,
  focusedPaneId,
  layoutHasMultiplePanes,
  layoutRoot,
  paneRouter,
  layoutPaneIds,
  setSplitRatio,
  type LayoutNode,
} from "./panes";
import { paneSel, samePaneTarget } from "./paneSelect";
import { SurfaceContext } from "./components/Block";
import { endEdit } from "./editorController";

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
        layoutPaneIds().includes(p.paneId) && sameRoute(paneRouter(p.paneId).route(), p.route)
      ),
  };
}

function requestJournalFeedWatcherRestart(
  routes: Array<{ paneId: string; route: ReturnType<PaneRouter["route"]> }>
) {
  const owner = journalsFeedOwner(routes);
  if (owner) void reloadJournalsFeedFromStart(owner);
}

export async function handleGraphChange(c: GraphChange) {
  // The backend watcher has already landed this transaction in its graph cache.
  // Invalidate every derived visible-entity view even when the changed page is
  // outside the bounded frontend working set (#166); loaded pages are refreshed
  // below, while unloaded block references re-resolve by UUID from dataRev.
  bumpDataRev();
  const routes = layoutPaneIds().map((paneId) => ({ paneId, router: paneRouter(paneId), route: paneRouter(paneId).route() }));
  if (c.removed) {
    const disp = reloadDisposition(c.name);
    if (disp === "conflict") {
      markConflict(c.name);
      if (c.kind === "journal") requestJournalFeedWatcherRestart(routes);
      return;
    }
    if (disp === "skip") {
      if (c.kind === "journal") requestJournalFeedWatcherRestart(routes);
      return;
    }
    for (const p of routes) {
      if (p.route.kind === "page" && p.route.name === c.name) {
        if (p.router.canGoBack()) p.router.goBack();
        else if (!closePane(p.paneId)) p.router.openJournals({ inPlace: true });
      }
    }
    if (c.kind === "journal" && routes.some((p) => p.route.kind === "journals")) {
      restoreTodayJournalInFeed();
      requestJournalFeedWatcherRestart(routes);
    }
    return;
  }

  const disp = reloadDisposition(c.name);
  if (disp === "skip") {
    if (c.kind === "journal") requestJournalFeedWatcherRestart(routes);
    return;
  }
  if (disp === "conflict") {
    markConflict(c.name);
    if (c.kind === "journal") requestJournalFeedWatcherRestart(routes);
    return;
  }
  if (routes.some((p) => p.route.kind === "page" && p.route.name === c.name)) {
    const dto = await backend().getPage(c.name, c.kind);
    if (dto) reloadPage(toLoadablePage(dto, c.name));
    // A page surface may have the same journal loaded while another live pane
    // shows Journals.  Reloading that DTO is not feed reconciliation: always
    // give the live feed owner its authoritative null-cursor restart too.
    if (c.kind === "journal") requestJournalFeedWatcherRestart(routes);
    return;
  }
  if (c.kind === "journal" && routes.some((p) => p.route.kind === "journals")) {
    if (pageByName(c.name)) {
      const dto = await backend().getPage(c.name, c.kind);
      if (dto) reloadPage(dto);
      requestJournalFeedWatcherRestart(routes);
      return;
    }
    // The feed owner performs the page-scoped dirty/save/conflict/move gate.
    // Calling it even while unsafe records a pending restart instead of losing
    // this watcher update until another unrelated file changes.
    requestJournalFeedWatcherRestart(routes);
    return;
  }
  if (pageByName(c.name) && !doc.feed.includes(c.name)) {
    const dto = await backend().getPage(c.name, c.kind);
    if (dto) reloadPage(dto);
  }
}

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
  return (
    <div
      class={`pane-resizer pane-resizer-${props.dir}`}
      classList={{ "pane-seam-selected": samePaneTarget(paneSel(), { kind: "seam", path: props.path }) }}
      data-pane-seam-path={props.path.join(".")}
      data-pane-seam-dir={props.dir}
      onPointerDown={(e) => {
        e.preventDefault();
        const box = (e.currentTarget.parentElement as HTMLElement).getBoundingClientRect();
        const onMove = (ev: PointerEvent) => {
          const raw =
            props.dir === "row"
              ? (ev.clientX - box.left) / Math.max(1, box.width)
              : (ev.clientY - box.top) / Math.max(1, box.height);
          setSplitRatio(props.path, raw);
        };
        const onUp = () => {
          window.removeEventListener("pointermove", onMove);
          window.removeEventListener("pointerup", onUp);
        };
        window.addEventListener("pointermove", onMove);
        window.addEventListener("pointerup", onUp);
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
    <Show
      when={props.router.route().kind === "query"}
      fallback={<PageView />}
    >
      <QueryWorkspace route={props.router.route() as QueryRoute} router={props.router} />
    </Show>
  );
}

function PaneLeaf(props: { paneId: string }): JSX.Element {
  const router = paneRouter(props.paneId);
  const multi = () => layoutHasMultiplePanes();
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
              <main
                class="main-content"
                data-pane-id={props.paneId}
                ref={(el) => router.setScrollerElement(el)}
              >
                <div class="main-content-inner">
                  <PaneContent router={router} />
                </div>
              </main>
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
            <TabBar
              router={router}
              dragRegion={false}
              paneStrip
              focused={focusedPaneId() === props.paneId}
            />
            <main class="main-content pane-main-content" ref={(el) => router.setScrollerElement(el)}>
              <div class="main-content-inner">
                <PaneContent router={router} />
              </div>
            </main>
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
  return (
    <Show when={paneSel()}>
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

export function PaneEdgeHighlights(): JSX.Element {
  const edge = () => {
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

export async function installMobileExternalLinkHandler(): Promise<() => void> {
  if ((await backend().appPlatform()) === "desktop") return () => {};

  const onClick = (e: MouseEvent) => {
    const target = e.target;
    const el = target instanceof Element ? target : target instanceof Node ? target.parentElement : null;
    const a = el?.closest?.("a[href]") as HTMLAnchorElement | null;
    const href = a?.getAttribute("href")?.trim() ?? "";
    if (!a || !/^(https?:\/\/|mailto:)/i.test(href)) return;

    e.preventDefault();
    e.stopPropagation();
    void backend().openExternal(a.href);
  };

  document.addEventListener("click", onClick, true);
  return () => document.removeEventListener("click", onClick, true);
}

export function App(): JSX.Element {
  // Startup debug trace (TINE_DEBUG=1 / --debug): forward UI milestones + errors
  // into the backend log so a remote "bad startup" is diagnosable in one file.
  onMount(() => void initDebug());

  // One-time notice after the desktop identifier rename chain
  // dev.tine.app / page.tine.app -> page.tine.Tine: the backend moved
  // settings/session/backups to the new app-data dir, but some app-level prefs
  // (window geometry, possibly shortcuts) may have reset. Sticky so the user
  // actually sees it; the backend flag self-clears after this one read.
  onMount(async () => {
    try {
      if (await backend().takeIdentifierMigrationNotice()) {
        pushToast(
          "Tine was renamed under the hood, so we moved your settings and backups across. A few app-level preferences (e.g. keyboard shortcuts) might need setting again — sorry about that!",
          "info",
          { sticky: true }
        );
      }
    } catch {
      // Non-Tauri/mock or an older backend without the command: nothing to notify.
    }
  });

  onMount(async () => {
    const injected = (window as any).__GRAPH_PATH__ ?? "";
    let startup = "";
    try {
      startup = (await backend().startupGraphPath()) ?? "";
    } catch {
      startup = "";
    }
    const graphPath = injected || startup || persistedGraphPath();
    dbg(`loading graph: ${graphPath || "(default/configured)"}`);
    try {
      await loadGraphPath(graphPath);
      dbg("graph load call returned");
    } catch (e) {
      // No graph configured (fresh install), or it failed to open. Fall through to
      // the onboarding Welcome screen instead of leaving a blank app; don't toast
      // on first run (the empty/`""` path legitimately has no graph yet).
      dbg(`graph load failed: ${String(e)}`);
    } finally {
      setFirstLoadDone(true);
    }
  });

  // Warn (loudly) if the webview is painting on the CPU — Tine's whole pitch is
  // speed, so a silent software-rendering fallback shouldn't read as "Tine is
  // slow". Fire-and-forget; the probe is Tauri-gated and never throws.
  onMount(() => void warnIfSoftwareRendering());

  // Once per launch, a few seconds after startup (so it never competes with the
  // first paint or the graph load), check GitHub for a newer release and toast if
  // there is one. Best-effort + silent on failure (see update.ts).
  onMount(() => {
    const t = setTimeout(() => void checkForUpdate(), 3000);
    onCleanup(() => clearTimeout(t));
  });

  // Re-install experimental smooth scrolling (Lenis) if it was left on. The feed
  // (`.main-content`) is mounted by now (onMount runs after first render).
  onMount(() => void initSmoothScroll());
  onMount(() => void initCopySettings());
  onMount(() => void initRefCompletionSettings());
  onMount(() => void initNavSettings());
  // Load the local-file images opt-in (Settings → Editing). Default off.
  onMount(() => void initLocalFileSettings());
  // A conflict copy appearing/vanishing on disk (watcher) refreshes the list.
  onMount(() => {
    let unsub = () => {};
    void backend()
      .onConflictsChanged(() => void refreshSyncConflicts())
      .then((u) => (unsub = u));
    onCleanup(() => unsub());
  });
  // One graph-file watcher for every pane. PageView instances render pane
  // content; they do not each own a backend subscription.
  onMount(() => {
    let unsub = () => {};
    void backend()
      .onGraphChanged((c) => void handleGraphChange(c))
      .then((u) => (unsub = u));
    onCleanup(() => unsub());
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
    void installMobileExternalLinkHandler().then((u) => {
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
        setGraphTransitioning(true);
        const active = document.activeElement;
        if (active instanceof HTMLElement) active.blur();
        endEdit("graph-switch");
        await Promise.resolve();
        // Try to persist everything. Cap the wait so a genuinely stuck save IPC
        // can't wedge the window open forever — but a timeout counts as "not
        // saved", not "safe to discard".
        let saved = false;
        try {
          saved = await Promise.race([
            flushAll(),
            new Promise<boolean>((r) => setTimeout(() => r(false), 4000)),
          ]);
        } catch {
          saved = false;
        }
        // If edits remain unsaved (conflict, error, or stalled flush), DON'T
        // silently throw them away — ask. Default (Cancel) keeps the app open so
        // the user can resolve the conflict; only an explicit confirm quits.
        if (!saved) {
          // Native GTK confirm — window.confirm silently returns true in this
          // WebKitGTK build, which would quit and discard the unsaved edits with
          // no prompt at all. The whole point here is to NOT lose them silently.
          let quit = false;
          try {
            quit = await backend().confirm(
              "Tine has unsaved changes that couldn't be saved (a conflict or a stuck save).\n\nClose this window anyway and lose them?",
              "Unsaved changes"
            );
          } catch {
            pushToast("Couldn't confirm closing the window. Your unsaved changes are still open.", "error");
            closeInProgress = false;
            setGraphTransitioning(false);
            return;
          }
          if (!quit) {
            closeInProgress = false;
            setGraphTransitioning(false);
            return; // stay open
          }
        }
        // Persist the final tab session too — the 150ms debounce may not have
        // fired if the last tab action came right before quitting. Capped so a
        // stuck IPC can't wedge the window open.
        try {
          await Promise.race([flushSession(), new Promise((r) => setTimeout(r, 1000))]);
        } catch {
          // best-effort
        }
        allowClose = true;
        // Close only this graph window. The backend exits the process (including
        // Linux WebKit cleanup) only when this is the final graph window.
        try {
          await backend().closeGraphWindow();
          return;
        } catch {
          // fall through to the direct close below
        }
        try {
          await w.destroy();
        } catch {
          await w.close(); // re-fires onCloseRequested; the guard lets it close
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
    let unlisten = () => {};
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
    void (async () => {
      const { emitTo, listen } = await import("@tauri-apps/api/event");
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const windowLabel = getCurrentWindow().label;
      const ack = (id: string | undefined, ok: boolean) => {
        if (id) void emitTo("capture", "quick-capture-ack", { id, ok } satisfies QuickCaptureAck);
      };
      unlisten = await listen<QuickCaptureRequest>("quick-capture", async (e) => {
        // WebKitGTK currently exposes targeted Tauri events to every graph
        // listener in this process. Treat the payload label as the authority so
        // only the selected graph can ever perform the write.
        if (e.payload?.target !== windowLabel) return;
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
            ok = title ? await captureToPage(title, text) : await appendToTodayJournal(text);
          } catch {
            ok = false;
          }
          pushToast(
            ok
              ? title
                ? `Captured to “${title}”`
                : "Captured to today's journal"
              : "Capture couldn't be saved",
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
      });
    })();
    onCleanup(() => unlisten());
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
    void (async () => {
      try {
        const { emitTo } = await import("@tauri-apps/api/event");
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        if ((await backend().captureTarget()) === getCurrentWindow().label) {
          await emitTo("capture", "capture-apply-theme", { theme: t });
        }
      } catch {
        // No graph is bound yet (Welcome) or capture is unavailable.
      }
    })();
  });

  // After edits settle (dataRev bumps), refresh the alias map so changing an
  // alias:: doesn't leave navigation resolving to the old canonical page. The
  // Rust side caches aliases, so this is cheap unless a save actually changed them.
  createEffect(on(dataRev, () => void refreshAliases(), { defer: true }));

  // (Re)install keybindings whenever config or the user's local overrides change
  // (precedence: defaults < config.edn :shortcuts < Settings overrides). We also
  // mirror the merged map to the quick-capture window so a remapped
  // editor/quick-capture-file (or any editor shortcut) is honored there too — it
  // can't read this window's localStorage overrides on its own.
  let latestShortcuts: Record<string, string> = {};
  const broadcastShortcuts = () => {
    if (!isTauri()) return;
    void (async () => {
      try {
        const { emitTo } = await import("@tauri-apps/api/event");
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        if ((await backend().captureTarget()) === getCurrentWindow().label) {
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
        <div class="parser-error-banner" role="alert">
          The block renderer failed to load — text is shown unformatted. Please reload Tine;
          if this persists, report it.
        </div>
      </Show>
      <Show when={graphTransitioning()}>
        <div class="graph-transition-shield" role="status" aria-live="polite">
          Finishing graph operation…
        </div>
      </Show>
      <Show when={sidebarOpen()}>
        <div class="left-sidebar" style={{ flex: `0 0 ${sidebarWidth()}px`, width: `${sidebarWidth()}px` }}>
          <div class="left-sidebar-scroll">
            <Sidebar />
          </div>
          <div
            class="sidebar-resizer"
            onMouseDown={(e) => {
              e.preventDefault();
              const onMove = (ev: MouseEvent) =>
                setSidebarWidth(Math.min(500, Math.max(180, ev.clientX)));
              const onUp = () => {
                window.removeEventListener("mousemove", onMove);
                window.removeEventListener("mouseup", onUp);
                persistSidebarWidth();
              };
              window.addEventListener("mousemove", onMove);
              window.addEventListener("mouseup", onUp);
            }}
          />
        </div>
      </Show>
      <div class="main-container">
        {/* In focus mode the topbar is hidden; this thin strip at the very top
            reveals it on hover (CSS adjacency), so controls are reachable. */}
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
              onClick={toggleSidebar}
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
              class="icon-btn"
              title="Go back"
              data-pane-focus-neutral
              disabled={!canGoBack()}
              onClick={goBack}
            >
              <svg viewBox="0 0 24 24" class="nav-icon">
                <path d="M15 5l-7 7 7 7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
            <button
              class="icon-btn"
              title="Go forward"
              data-pane-focus-neutral
              disabled={!canGoForward()}
              onClick={goForward}
            >
              <svg viewBox="0 0 24 24" class="nav-icon">
                <path d="M9 5l7 7-7 7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
          </div>
          {/* The tab strip is a desktop feature; on a phone it only crowds the
              single-row toolbar (and its pill clips). Hide it there, keeping a
              flex spacer so the right-side icons stay pinned to the edge. */}
          <Show when={!isMobilePlatform && !layoutHasMultiplePanes()} fallback={<div class="topbar-spacer" data-tauri-drag-region />}>
            {/* Keyed on the SOLE pane's id: after closing panes the survivor
                need not be "main", and TabBar freezes its router at mount. */}
            <Show when={firstPaneId(layoutRoot()) ?? "main"} keyed>
              {(soloId) => <TabBar router={paneRouter(soloId)} />}
            </Show>
          </Show>
          <div class="topbar-right">
            <CalendarJump />
            <button class="icon-btn" title="Journals" data-pane-focus-neutral onClick={() => openJournals()}>
              <svg viewBox="0 0 24 24" class="nav-icon">
                <path d="M4 5h11a2 2 0 0 1 2 2v12H6a2 2 0 0 1-2-2V5z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" />
                <line x1="8" y1="9" x2="14" y2="9" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
                <line x1="8" y1="12.5" x2="14" y2="12.5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
                <path d="M17 5h3v14a2 2 0 0 1-2 2 1 1 0 0 1-1-1V5z" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" />
              </svg>
            </button>
            <button class="icon-btn" title="Toggle theme (t t)" onClick={toggleTheme}>
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
              class="icon-btn"
              classList={{ active: rightSidebarOpen() }}
              title="Toggle right sidebar (t r)"
              onClick={toggleRightSidebar}
            >
              <svg viewBox="0 0 24 24" class="nav-icon">
                <rect x="3" y="4" width="18" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.7" />
                <line x1="15" y1="4" x2="15" y2="20" stroke="currentColor" stroke-width="1.7" />
              </svg>
            </button>
            {/* Settings sits apart at the far right (separated by a divider) so
                it reads as app-level config, not another content control. */}
            <span class="topbar-sep" />
            <button class="icon-btn" title="Settings (t s)" onClick={() => openSettings()}>
              <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
                <path
                  fill="currentColor"
                  d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.488.488 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 00-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 00-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.07.62-.07.94s.02.64.07.94l-2.03 1.58a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 00-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"
                />
              </svg>
            </button>
            {/* Frameless-window controls live at the very right, where the native
                title bar's buttons used to be. Hidden when the OS draws its own
                (macOS Overlay always; Linux/Windows when the native-frame toggle
                is on). */}
            <Show when={isTauri() && !osDrawsWindowControls()}>
              <span class="topbar-sep" />
              <WindowControls />
            </Show>
          </div>
        </header>
        <ConflictBar />
        <InPageFind />
        {/* Everything below the topbar lives in this row, so the topbar (and its
            window controls at the far right) spans the full window width and the
            right sidebar / PDF pane sit UNDER it — not beside the close button. */}
        <div class="content-row">
          <PaneEdgeHighlights />
          <PaneSelectHint />
          <PaneTree node={layoutRoot()} path={[]} />
          <RightSidebar />
          <Show when={pdfTarget()}>
        <div class="pdf-pane" data-pane-id="pdf" style={{ flex: `0 0 ${pdfPaneWidth()}px`, width: `${pdfPaneWidth()}px` }}>
          <div
            class="pdf-pane-resizer"
            onMouseDown={(e) => {
              e.preventDefault();
              const startX = e.clientX;
              const startW = pdfPaneWidth();
              const onMove = (ev: MouseEvent) =>
                setPdfPaneWidth(Math.min(1200, Math.max(320, startW + (startX - ev.clientX))));
              const onUp = () => {
                window.removeEventListener("mousemove", onMove);
                window.removeEventListener("mouseup", onUp);
                persistPdfPaneWidth();
              };
              window.addEventListener("mousemove", onMove);
              window.addEventListener("mouseup", onUp);
            }}
          />
          <Suspense fallback={<div class="pdf-loading" />}>
            <KeyedPdfViewer target={pdfTarget} />
          </Suspense>
        </div>
          </Show>
        </div>
      </div>
      <Show when={focusMode()}>
        <button class="focus-exit" title="Exit focus (Esc)" onClick={() => void exitFocusMode()}>
          <svg viewBox="0 0 24 24" class="nav-icon">
            <path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
          </svg>
        </button>
      </Show>
      <QuickSwitcher />
      <ContextMenu />
      <DatePicker />
      <FormulaEditor />
      <MobileKeyboardToolbar />
      <PageProps />
      <ExportModal />
      <PdfExportDialog />
      <Settings />
      <HelpPopup />
      {/* First-run onboarding: covers the (empty) app when no graph is configured.
          Rendered before Toasts so a "couldn't create graph" toast still shows on top. */}
      <Show when={welcomeOpen() || (globalThis as any).__FORCE_WELCOME__ === true || (firstLoadDone() && !graphMeta())}>
        <Welcome onClose={welcomeOpen() ? closeWelcome : undefined} />
      </Show>
      <Toasts />
      <Lightbox />
      <AudioOverlay />
      {/* Resize grips for the frameless window — hidden while maximized (no edge
          to drag, and they'd otherwise overlap the content scrollbar) and whenever
          the OS draws its own frame (it provides resize borders). */}
      <Show when={isTauri() && !osDrawsWindowControls() && !maximized()}>
        <ResizeGrips />
      </Show>
    </div>
  );
}
