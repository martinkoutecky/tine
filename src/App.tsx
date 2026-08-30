import { Match, Show, Suspense, Switch, createEffect, createMemo, createSignal, lazy, on, onCleanup, onMount, type JSX } from "solid-js";
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
import { WorkspaceSwitcher } from "./components/WorkspaceSwitcher";
import { TopbarOverflowMenu } from "./components/TopbarOverflowMenu";
import { ContextMenu } from "./components/ContextMenu";
import { Toasts, Lightbox } from "./components/Toasts";
import { AbsenceSweepCenter } from "./components/AbsenceSweepCenter";
import { AudioOverlay } from "./components/AudioOverlay";
import { CalendarJump } from "./components/CalendarJump";
import { ConflictBar } from "./components/ConflictBar";
import { RightSidebar } from "./components/RightSidebar";
// Settings pulls in the plugin/theme catalogues, backup controls, and every
// settings tab. Most launches never open it, so keep that work out of the
// startup bundle and fetch it only when the settings surface is requested.
const Settings = lazy(() =>
  import("./components/Settings").then((m) => ({ default: m.Settings }))
);
import { HelpPopup } from "./components/HelpShortcuts";
import { DatePicker } from "./components/DatePicker";
import { FormulaEditor } from "./components/FormulaEditor";
import { MobileKeyboardToolbar } from "./components/MobileKeyboardToolbar";
import {
  DrawerBackground,
  MobileDrawerController,
  MobileDrawerPanel,
  dismissDrawerAndRestore,
} from "./components/MobileDrawerShell";
import { PageProps } from "./components/PageProps";
import { ExportModal } from "./components/ExportModal";
import { PdfExportDialog } from "./components/PdfExportDialog";
import { StartupRecoveryLayer } from "./components/StartupRecovery";
import { InPageFind } from "./components/InPageFind";
import { installKeybindings } from "./keybindings";
import { installFileDrop } from "./filedrop";
import { installBlockSelectionDrag } from "./blockDrag";
import { applyGraphConfigChange, loadGraphPath, persistedGraphPath, refreshAliases, refreshPageIdentities, switchGraph } from "./graph";
import { applyObservedAssetChanges } from "./assetRefresh";
import { favoritesPageChanged } from "./favoritesStore";
import { checkForUpdate } from "./update";
import { WelcomeLayer } from "./components/Welcome";
import { goBack, goForward, canGoBack, canGoForward, flushSession, openJournals, sameRoute, type PaneRouter, type QueryRoute } from "./router";
import {
  theme,
  toggleTheme,
  sidebarOpen,
  toggleSidebar,
  rightSidebarOpen,
  toggleRightSidebar,
  openSwitcher,
  sidebarWidth,
  setSidebarWidth,
  persistSidebarWidth,
  graphMeta,
  firstLoadDone,
  setFirstLoadDone,
  openSettings,
  settingsOpen,
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
  pageInventoryRev,
  bumpPageInventoryRev,
  installPaneTracker,
  isConflicted,
  conflicts,
  pushToast,
  refreshSyncConflicts,
  refreshConflictQueueIfTouched,
  graphEpoch,
  graphTransitioning,
  setGraphTransitioning,
  activeDrawer,
  completeActiveLeftNavigation,
  dismissMobileDrawer,
} from "./ui";
import { mobileDrawerMode, restoreDrawerFocus } from "./mobileDrawers";
import { dismissTopTransient } from "./transientLayers";
import { applyZoom, installInterfaceZoomKeys, installInterfaceZoomWheel } from "./zoom";
import {
  doc,
  flushAll,
  appendToTodayJournal,
  captureToPage,
  deferExternalReload,
  installExternalReloadReplayHandler,
  pageByName,
  focusFreshnessPageNames,
  reloadDisposition,
  reloadPageIfStillSafe,
  restoreTodayJournalInFeed,
} from "./store";
import {
  applyDivergenceVerdict,
  conflictObservationKindFor,
  graphBinding,
  isSaving,
  reconcileExternalChange,
  saveBaselineFor,
} from "./persistence";
import type { QuickCaptureAck, QuickCaptureRequest } from "./quickCaptureAck";
import { backend, isTauri, type GraphChange, type GraphChangedBulk } from "./backend";
import { parserFailed } from "./render/parse";
import { warnIfSoftwareRendering } from "./gpu";
import { initSmoothScroll } from "./smoothScroll";
import { initCopySettings } from "./copySettings";
import { initRefCompletionSettings } from "./refCompletionSettings";
import { initNavSettings } from "./navSettings";
import { initLocalFileSettings } from "./localFileSettings";
import {
  conflictPolicyAlwaysAsk,
  holdExternalChange,
  initConflictPolicy,
  installHeldExternalChangeApplier,
  setConflictPolicyAlwaysAskForTest,
} from "./conflictPolicy";
import { initAssetSettings } from "./assetSettings";
import { initMediaEditorSettings } from "./mediaEditorSettings";
import { initSpellcheckSettings } from "./spellcheckSettings";
import { initLinkDefault } from "./editor/linkDefault";
import { dbg, initDebug } from "./debug";
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
  openPdfNotes,
  layoutPaneIds,
  setSplitRatio,
  visibleLayoutNode,
  type LayoutNode,
} from "./panes";
import { paneSel, samePaneTarget } from "./paneSelect";
import { SurfaceContext } from "./components/Block";
import { endEdit } from "./editorController";
import { installBackgroundFlush } from "./backgroundFlush";
import {
  installFocusFreshnessVerifier,
  installReloadOnFocus,
  trackGraphChangeApplication,
} from "./reloadOnFocus";
import { freshnessVisible } from "./freshnessBarrier";
import { createAndroidRootCloseCoordinator, exitAndroidActivity, installAndroidBackHandler } from "./androidBack";
import { createSafeCloseCoordinator } from "./safeClose";
import { drainPdfWork } from "./pdfOwnership";
import { currentPdfOwnership } from "./pdfOwnership";
import { hlsPageName } from "./pdf";
import { managedStorageRuntime, managedStorageRuntimeErrorMessage } from "./managedStorageRuntime";
import { createStartupRecoveryController } from "./startupRecovery";
import { storageTransitionRuntime } from "./storageTransitionRuntime";
import { writeClipboardTextResilient } from "./clipboard";
import type { SparseV2CancelResult } from "./types";
import {
  rebindAbsenceSweepScope,
  ingestAbsenceSweepEvent,
} from "./absenceSweeps";

/** The single persistence transaction used by both desktop close and Android
 * root Back.  Callers choose only the final platform action. */
const safeClose = createSafeCloseCoordinator({
  blurActive() {
    const active = document.activeElement;
    if (active instanceof HTMLElement) active.blur();
  },
  endEdit() {
    endEdit("graph-switch");
  },
  flushPdfWork: drainPdfWork,
  flushAll,
  confirmDiscard: (reason) => backend().confirm(
    reason === "still-saving"
      ? "Tine is still writing your changes and is taking longer than expected — a slow or network drive can do this.\n\nClosing now would lose whatever hasn't been written yet. Close anyway?"
      : "Tine has unsaved changes that couldn't be saved (a conflict or a stuck save).\n\nClose this window anyway and lose them?",
    "Unsaved changes",
  ),
  flushSession,
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

const androidRootClose = createAndroidRootCloseCoordinator(safeClose, {
  prepareNativeClose: () => backend().prepareQuit(),
  finishActivity: exitAndroidActivity,
  nativePrepareFailed: (failure) => pushToast(
    failure.status === "refused" || failure.status === "partial"
      ? "Tine-managed storage could not verify a clean stop. The app remains open so you can retry or inspect recovery status."
      : "Couldn't close the app. Your graph remains open.",
    "error",
  ),
  finishActivityFailed: () => pushToast(
    "Tine safely stopped managed storage but couldn't close the Android activity. Tap Back to retry closing.",
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
  const binding = graphBinding();
  const owners = routes.filter((p) => p.route.kind === "journals");
  if (!owners.length) return null;
  return {
    graphEpoch: epoch,
    graphBinding: binding,
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

// A skip/decline below records the change for deferred replay; the replay
// re-enters this same handler so the disposition is re-evaluated with whatever
// state holds at that moment (it may have become "conflict", which then takes
// the divergence path exactly like a live event).
installExternalReloadReplayHandler((change) => void handleGraphChange(change));

// Native rescan completion means the backend cache is current, but event
// callbacks cross the Tauri bridge independently. Verify the bounded set of
// pages the user can immediately interact with against that cache before the
// focus input barrier opens. This is intentionally O(active pages), not
// O(graph), and reuses the ordinary external-change policy below.
installFocusFreshnessVerifier(async () => {
  const binding = graphBinding();
  const changes: GraphChange[] = [];
  for (const name of focusFreshnessPageNames()) {
    const loaded = pageByName(name);
    if (!loaded) continue;
    const current = loaded.path
      ? await backend().getPageByPath(loaded.path)
      : await backend().getPage(loaded.name, loaded.kind);
    if (binding !== graphBinding()) return;
    const baseline = saveBaselineFor(name);
    const currentRev = current?.rev ?? null;
    if (currentRev === baseline) continue;
    changes.push({
      name,
      kind: loaded.kind,
      created: baseline === null && current !== null,
      removed: current === null,
    });
  }
  if (!changes.length || binding !== graphBinding()) return;
  bumpDataRev();
  if (changes.some((change) => change.created || change.removed)) {
    bumpPageInventoryRev();
  }
  for (const change of changes) {
    if (binding !== graphBinding()) return;
    await applyExternalChange(change, binding);
  }
});

// Concord L0's reload-on-focus fallback. Returning to the window replays
// anything already deferred and asks the backend watcher for one full stat diff,
// for the filesystems and sync clients that give us no event at all. Both halves
// funnel into the machinery above; neither applies anything by itself.
installReloadOnFocus();

// Read BEFORE the router normalizes the URL on load (the mock reads `?conflicts`
// at call time instead, which is why that gate needs no snapshot).
const ALWAYS_ASK_DEMO =
  typeof location !== "undefined" && /[?&]alwaysask\b/.test(location.search);
const ABSENCE_SWEEP_DEMO =
  typeof location !== "undefined" && /[?&]absence-sweeps\b/.test(location.search);

// Concord P5 policy toggle: "Reload from disk" on a held change re-enters the
// ordinary external-change path with the policy bypassed for that one change, so
// every other gate (disposition, editor leases, deferred replay) still applies.
installHeldExternalChangeApplier((change, binding) => {
  void applyExternalChange(change, binding, { bypassPolicy: true });
});

// Console-only diagnostic for external-change latency reports (GH #337; see
// docs/concord.md). Release builds ship the devtools but not `withGlobalTauri`,
// so a reporter needs one named callable to reach the backend's receipt ring.
// No UI beyond this.
if (isTauri()) {
  (window as unknown as {
    __tineWatcherLatency?: () => Promise<unknown>;
  }).__tineWatcherLatency = async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke("watcher_latency_recent");
  };
}

export async function handleGraphChange(c: GraphChange) {
  const binding = graphBinding();
  // The backend watcher has already landed this transaction in its graph cache.
  // Invalidate every derived visible-entity view even when the changed page is
  // outside the bounded frontend working set (#166); loaded pages are refreshed
  // below, while unloaded block references re-resolve by UUID from dataRev.
  bumpDataRev();
  if (c.created || c.removed) bumpPageInventoryRev();
  // The favorites arrangement page is an ordinary page, so an outside edit to
  // it arrives here like any other — and the sidebar has to follow it.
  void favoritesPageChanged([c.name]);
  await applyExternalChange(c, binding);
  // A merge finished outside Tine (git resolving the markers, a sync tool
  // removing a copy) must not leave a stale item in the conflict queue.
  await refreshConflictQueueIfTouched([c]);
}

/** The per-page half of `handleGraphChange`: everything except the dataRev /
 *  inventory bumps, which a bulk revision performs once for its whole epoch.
 *  `suppressFeedRestart` lets that bulk path restart a live Journals feed once
 *  at the end instead of once per changed journal. */
async function applyExternalChange(
  c: GraphChange,
  binding: number,
  opts: { suppressFeedRestart?: boolean; bypassPolicy?: boolean } = {},
) {
  const routes = layoutPaneIds().map((paneId) => ({ paneId, router: paneRouter(paneId), route: paneRouter(paneId).route() }));
  const requestJournalFeedRestart = (
    owned: Array<{ paneId: string; route: ReturnType<PaneRouter["route"]> }>
  ) => {
    if (!opts.suppressFeedRestart) requestJournalFeedWatcherRestart(owned);
  };
  if (c.removed) {
    const disp = reloadDisposition(c.name);
    if (disp === "conflict") {
      // The file is gone under an unsaved edit. Let the guarded save decide and
      // raise the banner: only its refusal carries the authority "Keep mine"
      // must present (see `reconcileExternalChange`).
      await reconcileExternalChange(c.name);
      if (c.kind === "journal") requestJournalFeedRestart(routes);
      return;
    }
    if (disp === "skip") {
      deferExternalReload(c, binding);
      if (c.kind === "journal") requestJournalFeedRestart(routes);
      return;
    }
    for (const p of routes) {
      if (p.route.kind === "page" && p.route.name === c.name) {
        if (p.router.canGoBack()) p.router.goBack();
        else if (!closePane(p.paneId)) p.router.openJournals({ inPlace: true });
      }
    }
    if (c.kind === "journal" && routes.some((p) => p.route.kind === "journals")) {
      await restoreTodayJournalInFeed();
      requestJournalFeedRestart(routes);
    }
    return;
  }

  const disp = reloadDisposition(c.name);
  if (disp === "skip") {
    deferExternalReload(c, binding);
    if (c.kind === "journal") requestJournalFeedRestart(routes);
    return;
  }
  // Concord P5 — "always ask". Reached only AFTER the skip/conflict branches, so
  // it converts the one SILENT case (a loaded, clean page) into an asked one and
  // changes nothing that already asked or deferred. A page Tine does not hold has
  // nothing to ask about: navigation refetches from the backend anyway.
  if (!opts.bypassPolicy && conflictPolicyAlwaysAsk() && disp === "reload" && pageByName(c.name)) {
    holdExternalChange(c, binding);
    return;
  }
  if (disp === "conflict") {
    // The page has an unsaved edit. A watcher event proves this page's FILE was
    // written, not that it was written to anything other than what we already
    // hold: Tine's own save normally suppresses its echo, but a synced/polled
    // graph or a self-write-marker gap still surfaces one (see store.upsertPage).
    // Require the same per-page divergence proof as the managed path — a false
    // conflict here blocks every subsequent save of the very edit it warns about.
    if (!isSaving(c.name)) {
      const current = await backend().getPage(c.name, c.kind);
      if (binding !== graphBinding()) return;
      await applyDivergenceVerdict(c.name, { exists: !!current, rev: current?.rev ?? null });
    }
    if (c.kind === "journal") requestJournalFeedRestart(routes);
    return;
  }
  if (routes.some((p) => p.route.kind === "page" && p.route.name === c.name)) {
    const dto = await backend().getPage(c.name, c.kind);
    // A decline here (an editor lease took hold, or the page turned dirty during
    // the await) is the same dropped-reload hole as "skip": defer, don't drop.
    if (dto && !(await reloadPageIfStillSafe(c.name, toLoadablePage(dto, c.name), binding))) {
      deferExternalReload(c, binding);
    }
    // A page surface may have the same journal loaded while another live pane
    // shows Journals.  Reloading that DTO is not feed reconciliation: always
    // give the live feed owner its authoritative null-cursor restart too.
    if (c.kind === "journal") requestJournalFeedRestart(routes);
    return;
  }
  if (c.kind === "journal" && routes.some((p) => p.route.kind === "journals")) {
    if (pageByName(c.name)) {
      const dto = await backend().getPage(c.name, c.kind);
      if (dto && !(await reloadPageIfStillSafe(c.name, dto, binding))) {
        deferExternalReload(c, binding);
      }
      requestJournalFeedRestart(routes);
      return;
    }
    // The feed owner performs the page-scoped dirty/save/conflict/move gate.
    // Calling it even while unsafe records a pending restart instead of losing
    // this watcher update until another unrelated file changes.
    requestJournalFeedRestart(routes);
    return;
  }
  if (pageByName(c.name) && !doc.feed.includes(c.name)) {
    const dto = await backend().getPage(c.name, c.kind);
    if (dto && !(await reloadPageIfStillSafe(c.name, dto, binding))) {
      deferExternalReload(c, binding);
    }
  }
}

/** An external bulk revision — a VCS checkout, branch switch, or big sync that
 *  the watcher coalesced into one `graph-changed-bulk` epoch (Concord P2).
 *
 *  One epoch, one invalidation: dataRev and the page inventory bump once for
 *  the whole batch. Only pages that need active handling are touched — visible
 *  (routed) pages reload through the existing safe path, and pages with unsaved
 *  state run exactly the same divergence/defer machinery as a single watcher
 *  event (a bulk change while a page is being edited defers that page's reload
 *  like any other). Everything else is left for lazy reload: navigation always
 *  refetches from the backend, whose cache the watcher has already updated.
 *  The user sees one calm summary toast — never a dialog. */
export async function handleGraphChangedBulk(bulk: GraphChangedBulk) {
  const binding = graphBinding();
  const changes = bulk.changes;
  if (!changes.length) return;
  bumpDataRev();
  if (changes.some((c) => c.created || c.removed)) bumpPageInventoryRev();
  // Outside the per-page loop below, which deliberately skips pages nothing is
  // showing: the arrangement page is normally one of those.
  void favoritesPageChanged(changes.map((c) => c.name));
  const routedNames = new Set<string>();
  for (const paneId of layoutPaneIds()) {
    const route = paneRouter(paneId).route();
    if (route.kind === "page") routedNames.add(route.name);
  }
  let conflicts = 0;
  for (const c of changes) {
    if (binding !== graphBinding()) return;
    const active = routedNames.has(c.name) || reloadDisposition(c.name) !== "reload";
    if (!active) continue;
    // Journal-feed restarts are suppressed per page and issued once below.
    await applyExternalChange(c, binding, { suppressFeedRestart: true });
    if (isConflicted(c.name)) conflicts += 1;
  }
  if (binding !== graphBinding()) return;
  if (changes.some((c) => c.kind === "journal")) {
    requestJournalFeedWatcherRestart(
      layoutPaneIds().map((paneId) => ({ paneId, route: paneRouter(paneId).route() }))
    );
  }
  // A bulk revision is exactly the shape that resolves marker conflicts (a
  // `git merge --continue`, a branch switch): re-derive the queue if it touched
  // anything queued.
  await refreshConflictQueueIfTouched(changes);
  const summary = `${changes.length} page${changes.length === 1 ? "" : "s"} updated externally`;
  const conflictSuffix = conflicts
    ? ` · ${conflicts} conflict${conflicts === 1 ? "" : "s"} to review`
    : "";
  pushToast(summary + conflictSuffix, "info");
}

export async function handleSparseV2Changed() {
  const binding = graphBinding();
  // Managed reconciliation reports one admitted aggregate epoch rather than
  // legacy per-file changes. Refresh only live surfaces and invalidate the
  // bounded inventory; unloaded pages remain demand-loaded from SQLite.
  bumpDataRev();
  bumpPageInventoryRev();
  const routes = layoutPaneIds().map((paneId) => ({
    paneId,
    route: paneRouter(paneId).route(),
  }));
  const refreshed = new Set<string>();
  for (const { route } of routes) {
    if (route.kind !== "page" || refreshed.has(`${route.pageKind}:${route.name}`)) continue;
    refreshed.add(`${route.pageKind}:${route.name}`);
    const disposition = reloadDisposition(route.name);
    if (disposition === "skip") continue;
    // A save already in flight needs no notification: its own `base_rev` guard is
    // the authority, and it decides against the exact bytes it is writing. Waking
    // it with an aggregate epoch can only produce a verdict on staler evidence.
    if (disposition === "conflict" && isSaving(route.name)) continue;
    const dto = await backend().getPage(route.name, route.pageKind);
    if (binding !== graphBinding()) return;
    if (disposition === "conflict") {
      // The page has an unsaved edit. Prove this page actually diverged before
      // blocking its saves — the epoch alone says only that SOMETHING was
      // admitted, which is usually our own write coming back — and lift an
      // existing conflict when the proof comes back negative.
      await applyDivergenceVerdict(route.name, { exists: !!dto, rev: dto?.rev ?? null });
      continue;
    }
    if (dto) await reloadPageIfStillSafe(route.name, toLoadablePage(dto, route.name), binding);
  }
  requestJournalFeedWatcherRestart(routes);
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
      <QueryWorkspace route={props.router.route() as QueryRoute} router={props.router} focusSource={focusedPaneId() === props.router.paneId} />
    </Show>
  );
}

function PaneRouteBody(props: {
  paneId: string;
  router: PaneRouter;
  scrollerClass?: string;
  identifyPane?: boolean;
}): JSX.Element {
  const route = () => props.router.route();
  createEffect(() => {
    if (route().kind === "pdf" || route().kind === "invalid") {
      props.router.setScrollerElement(null);
    }
  });
  return (
    <Switch
      fallback={
        <PaneScroller
          paneId={props.paneId}
          router={props.router}
          class={props.scrollerClass}
          identifyPane={props.identifyPane}
        >
          <PaneContent router={props.router} />
        </PaneScroller>
      }
    >
      <Match when={route().kind === "pdf" ? route() : null} keyed>
        {(pdfRoute) => (
          <div
            class="pdf-pane pdf-route-pane"
            classList={{ "pdf-pane-mobile": isMobilePlatform }}
            data-pane-id={props.identifyPane === false ? undefined : props.paneId}
            data-pdf-view-id={pdfRoute.kind === "pdf" ? pdfRoute.viewId : undefined}
          >
            <Suspense fallback={<div class="pdf-loading" />}>
              <KeyedPdfViewer
                route={() => pdfRoute as Extract<ReturnType<PaneRouter["route"]>, { kind: "pdf" }>}
                owner={currentPdfOwnership}
                focused={() => focusedPaneId() === props.paneId}
                onClose={() => { void props.router.closePdf(); }}
                onOpenNotes={(block?: string) => {
                  const current = props.router.route();
                  if (current.kind === "pdf") openPdfNotes(props.paneId, hlsPageName(current.filename), block);
                }}
              />
            </Suspense>
          </div>
        )}
      </Match>
      <Match when={route().kind === "invalid" ? route() : null} keyed>
        {(invalidRoute) => (
          <div class="pane-route-error" role="alert">
            <h2>{invalidRoute.kind === "invalid" ? invalidRoute.title : "Unavailable tab"}</h2>
            <p>{invalidRoute.kind === "invalid" ? invalidRoute.message : "This tab could not be restored."}</p>
            <button type="button" onClick={() => { void props.router.closeTab(props.router.activeId()); }}>Close tab</button>
          </div>
        )}
      </Match>
    </Switch>
  );
}

/** A page pane's end slack is a property of its natural content geometry, not
 *  of whether a textarea happens to be mounted.  Long pages keep the same
 *  breathing room in read and edit mode; fitting dashboard panes keep none.
 *  This makes click/edit transitions height-stable (GH #390) while retaining
 *  the pane-relative tail affordance from GH #369. */
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
    const frame = requestAnimationFrame(measure);
    if (typeof ResizeObserver === "undefined") {
      onCleanup(() => cancelAnimationFrame(frame));
      return;
    }
    const observer = new ResizeObserver(measure);
    observer.observe(scroller);
    observer.observe(inner);
    onCleanup(() => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    });
  });
  onMount(() => {
    if (isTauri() || !ABSENCE_SWEEP_DEMO) return;
    ingestAbsenceSweepEvent({
      sweep_id: "11111111-1111-4111-8111-111111111111",
      tier: "tier3",
      absence_count: 8,
      pages_at_open: 64,
      opened_at_unix_ms: 1_777_000_000_000,
      closed_at_unix_ms: 1_777_000_060_000,
      grace_deadline_unix_ms: 1_777_000_360_000,
      disposed_at_unix_ms: null,
      members: [
        { page_id: "1", path: "pages/Project roadmap.md" },
        { page_id: "2", path: "journals/2026_08_28.md" },
        { page_id: "3", path: "pages/Meeting notes.md" },
        { page_id: "4", path: "research/Reading queue.org" },
        { page_id: "5", path: "pages/Release checklist.md" },
        { page_id: "6", path: "journals/2026_08_27.md" },
        { page_id: "7", path: "pages/Ideas.md" },
        { page_id: "8", path: "pages/Archive index.md" },
      ],
      latest_action: null,
    }, { announce: true });
  });
  // One generation-scoped listener carries durable absence-sweep snapshots to
  // the global recovery surface. Rebinding clears the old graph's list; panel
  // dismissal is intentionally unrelated to this lifecycle.
  //
  // Both inputs MUST be memos: a bare `() => snapshot().field` accessor inside
  // `on(...)` retriggers on every snapshot replacement even when the field
  // value is unchanged (Solid dedupes per signal, and the snapshot signal
  // holds a fresh object each status event). Restore's own completion emits
  // such events, so the unmemoized form re-fired here and — via the
  // unconditional clear it used to call — closed the recovery panel at the
  // exact moment a Restore finished. The clear itself is additionally scoped
  // to real generation changes inside rebindAbsenceSweepScope.
  const sweepScopeGeneration = createMemo(
    () => managedStorageRuntime.snapshot().bindingGeneration,
  );
  const sweepScopeAuthority = createMemo(
    () => managedStorageRuntime.snapshot().applicationPageAdmission?.authority,
  );
  createEffect(on(
    [sweepScopeGeneration, sweepScopeAuthority],
    ([bindingGeneration, authority]) => {
      if (!isTauri() && ABSENCE_SWEEP_DEMO) return;
      rebindAbsenceSweepScope(bindingGeneration);
      if (bindingGeneration === null || authority !== "managed_writable") return;
      let disposed = false;
      let unlisten = () => {};
      void (async () => {
        try {
          const stop = await backend().onAbsenceSweepChanged(
            bindingGeneration,
            (sweep) => ingestAbsenceSweepEvent(sweep, { announce: true }),
          );
          if (disposed) {
            stop();
            return;
          }
          unlisten = stop;
        } catch {
          // The current snapshot remains useful if native event registration
          // is temporarily unavailable.
        }
        const sweeps = await backend().listAbsenceSweeps();
        if (disposed) return;
        for (const sweep of sweeps) ingestAbsenceSweepEvent(sweep, { announce: true });
      })().catch(() => {});
      onCleanup(() => {
        disposed = true;
        unlisten();
      });
    },
  ));

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
            <TabBar
              router={router}
              dragRegion={false}
              paneStrip
              focused={focusedPaneId() === props.paneId}
            />
            <PaneRouteBody paneId={props.paneId} router={router} scrollerClass="pane-main-content" identifyPane={false} />
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

/** Install the native post-cancel route before publishing its status.  In
 * particular, Direct Files must never briefly appear as a synthetic
 * managed-unavailable binding during cold recovery. */
export function acceptColdReturnManagedStorage(result: SparseV2CancelResult): void {
  managedStorageRuntime.clear();
  managedStorageRuntime.bind(
    result.binding_generation,
    result.status.application_page_admission,
  );
  managedStorageRuntime.receiveStatus(result.status);
}

export function App(): JSX.Element {
  let openCalendarJump = () => {};
  const topbarActions = {
    calendar: () => openCalendarJump(),
    journals: () => openJournals(),
    theme: () => toggleTheme(),
    rightSidebar: (trigger?: HTMLElement | null) => toggleRightSidebar(trigger),
    back: () => goBack(),
    forward: () => goForward(),
  };
  const startupRecovery = createStartupRecoveryController({
    lookupGraphPath: () => backend().startupGraphPath(),
    injectedGraphPath: () => (window as any).__GRAPH_PATH__ ?? "",
    persistedGraphPath,
    openGraph: (path, supersedeCurrent) => loadGraphPath(path, { supersedeCurrent }),
    pickGraph: switchGraph,
    coldReturn: (path) => backend().cancelSparseV2Cold(path),
    acceptColdReturn: acceptColdReturnManagedStorage,
    copyText: writeClipboardTextResilient,
    notify: (message, kind) => pushToast(message, kind, kind === "error" ? { sticky: true } : undefined),
    completeFirstLoad: () => setFirstLoadDone(true),
  });
  // Startup debug trace (TINE_DEBUG=1 / --debug): forward UI milestones + errors
  // into the backend log so a remote "bad startup" is diagnosable in one file.
  onMount(() => void initDebug());

  // The sparse runtime can tick while Settings is closed. Subscribe once at the
  // app boundary; the shared bridge carries the matching graph generation into
  // both this feedback and the panel without component-owned duplicate listeners.
  onMount(() => {
    let disposed = false;
    let unlisten = () => {};
    void managedStorageRuntime.listen().then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
    onCleanup(() => {
      disposed = true;
      unlisten();
    });
  });
  // One calm report per condition, not one per retry. The bridge advances the
  // notice sequence only for a message the user has not already been shown, and
  // clears it when the actor genuinely recovers, so a permanently blocked
  // reconciliation says its piece once and leaves Storage & sync to carry the
  // live status (GH: Android, 2026-08-18).
  createEffect(on(
    () => managedStorageRuntime.snapshot().notice,
    (notice) => {
      if (notice) pushToast(managedStorageRuntimeErrorMessage(notice.message), "error");
    },
    { defer: true },
  ));

  // SafeBackPlugin is the single Android native Back owner. A drawer/transient
  // is never represented by synthetic history; route history remains the JS
  // dispatch fallback once the native listener is explicitly ready.
  onMount(() => {
    if (!isTauri()) return;
    const uninstall = installAndroidBackHandler({
      platform: () => backend().appPlatform(),
      subscribe: async (handler) => {
        const { addPluginListener } = await import("@tauri-apps/api/core");
        return addPluginListener("safe-back", "android-safe-back", handler);
      },
      dismissTransient: () => dismissTopTransient("back"),
      dismissDrawer: () => dismissMobileDrawer("back"),
      restoreDrawerFocus: () => restoreDrawerFocus("back"),
      // The router's own back, not the WebView's: it knows whether Tine has an
      // entry to pop, and it is what every other Back affordance already uses.
      historyBack: () => {
        if (!canGoBack()) return false;
        goBack();
        return true;
      },
      closeRoot: () => { void closeAndroidRootSafely(); },
      // Listener absence/rejection remains owned by the native SafeBackPlugin,
      // which consumes Back rather than delegating to AppPlugin's unsafe
      // WebView/activity fallback.
      setupFailed: (error) => console.warn("Android SafeBack listener unavailable; native owner remains blocking", error),
    });
    onCleanup(uninstall);
  });

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

  // The normal app-data home was not writable, so this launch put settings, the
  // session and the WebView store somewhere else rather than crashing on the way
  // up. Sticky: the relocation lasts only as long as the permissions problem, so
  // the user needs to know where their state went and why.
  onMount(async () => {
    try {
      const fallback = await backend().takeDataHomeFallbackNotice();
      if (fallback) {
        pushToast(
          `Tine could not write its usual application-data folder, so this session is keeping settings and backups in ${fallback} instead. Fixing the permissions on that folder restores the normal location.`,
          "warn",
          { sticky: true }
        );
      }
    } catch {
      // Non-Tauri/mock or an older backend without the command: nothing to notify.
    }
  });

  onMount(() => {
    let disposed = false;
    let started = false;
    let unlistenStorage = () => {};
    let unlistenAssets = () => {};
    const start = () => {
      if (disposed || started) return;
      started = true;
      startupRecovery.start();
    };
    // Install both observation bridges before opening the graph. Native
    // managed-open phases can begin synchronously with the graph-open command,
    // while an image may render before the watcher has finished binding its
    // approved external-assets root. Starting after both listeners settle
    // prevents either early event from falling into a WebView subscription gap.
    void Promise.allSettled([
      backend().onStorageTransition((event) => {
        storageTransitionRuntime.receive(event);
        startupRecovery.receiveTransition(event);
      }),
      backend().onAssetChanged((batch) => {
        dbg(`asset-changed paths=${batch.paths.length}`);
        applyObservedAssetChanges(batch.paths);
      }),
    ]).then(([storage, assets]) => {
      if (storage.status === "fulfilled") unlistenStorage = storage.value;
      if (assets.status === "fulfilled") unlistenAssets = assets.value;
      if (disposed) {
        unlistenStorage();
        unlistenAssets();
        return;
      }
      // If either event bridge is unavailable, startup still attempts the
      // native command; command failure remains actionable without inventing a
      // timeout-based storage outcome.
      start();
    });
    onCleanup(() => {
      disposed = true;
      unlistenStorage();
      unlistenAssets();
      startupRecovery.dispose();
    });
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
  onMount(() => void initConflictPolicy());
  // Demo gate for the screenshot harness (mirrors `?conflicts`): turn the
  // always-ask policy on and hold one external change, so the bar is visible
  // without a real second writer. Browser mock only — never in the app.
  onMount(() => {
    if (isTauri() || !ALWAYS_ASK_DEMO) return;
    setConflictPolicyAlwaysAskForTest(true);
    (window as unknown as { __tineHoldExternalChange?: (name: string) => void })
      .__tineHoldExternalChange = (name: string) => {
      holdExternalChange(
        {
          name,
          kind: doc.pages.find((p) => p.name === name)?.kind ?? "page",
          created: false,
          removed: false,
        },
        graphBinding()
      );
    };
  });
  // A conflict copy appearing/vanishing on disk (watcher) refreshes the list.
  onMount(() => {
    let unsub = () => {};
    void backend()
      .onConflictsChanged(() => trackGraphChangeApplication(refreshSyncConflicts("new")))
      .then((u) => (unsub = u));
    onCleanup(() => unsub());
  });
  // One graph-file watcher for every pane. PageView instances render pane
  // content; they do not each own a backend subscription.
  onMount(() => {
    let unsub = () => {};
    void backend()
      .onGraphChanged((c) => trackGraphChangeApplication(handleGraphChange(c)))
      .then((u) => (unsub = u));
    onCleanup(() => unsub());
  });
  // Coalesced external bulk revisions (VCS checkout / big sync): one aggregate
  // event above the backend's bulk threshold instead of per-page events.
  onMount(() => {
    let unsub = () => {};
    void backend()
      .onGraphChangedBulk((bulk) => trackGraphChangeApplication(handleGraphChangedBulk(bulk)))
      .then((u) => (unsub = u));
    onCleanup(() => unsub());
  });
  // `logseq/config.edn` was rewritten outside Tine (Logseq, an editor, a sync
  // service) and the backend re-read it. Only settings changes arrive here; a
  // rewrite that moved nothing we surface emits nothing.
  onMount(() => {
    let unsub = () => {};
    void backend()
      .onGraphConfigChanged((meta) => applyGraphConfigChange(meta))
      .then((u) => (unsub = u));
    onCleanup(() => unsub());
  });
  onMount(() => {
    let unsub = () => {};
    void backend()
      .onSparseV2Changed(() => void handleSparseV2Changed())
      .then((u) => (unsub = u));
    onCleanup(() => unsub());
  });
  onMount(() => {
    let unsub = () => {};
    void backend()
      .onManagedSyncError(() => pushToast("Tine-managed storage stopped. Open Storage & sync to retry setup.", "error"))
      .then((u) => (unsub = u));
    onCleanup(() => unsub());
  });
  // A plain Markdown graph whose folder watch failed a reconcile cycle. Says
  // only what is known: Tine may miss outside changes until it recovers. It
  // deliberately makes no claim about whether saving still works — the guarded
  // write path has its own failure modes and this event does not measure them.
  onMount(() => {
    let unsub = () => {};
    void backend()
      .onGraphWatchError(() =>
        pushToast("Tine couldn't finish checking the graph folder for outside changes. It will keep retrying.", "error"),
      )
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
  // GH #255: the OS can reclaim a backgrounded app without ever sending a close
  // request, and everything inside the 400 ms save debounce is RAM-only until
  // then. This is the only durability barrier on Android/iOS, and it also covers
  // the desktop paths that skip a clean close. Installed unconditionally — it is
  // a DOM listener, so it works in the browser dev shell too.
  onMount(() => onCleanup(installBackgroundFlush({
    endEdit: () => endEdit("graph-switch"),
    flushAll,
    closeInFlight: () => safeClose.inFlight(),
  })));

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
        // Close only this graph window. The backend exits the process (including
        // Linux WebKit cleanup) only when this is the final graph window.
        try {
          await backend().closeGraphWindow();
          return;
        } catch (error) {
          if (String(error).includes("sparse-v2-shutdown-refused")) {
            allowClose = false;
            safeClose.reset();
            closeInProgress = false;
            pushToast(
              "Tine-managed storage could not verify a clean stop. The window remains open so you can retry or inspect recovery status.",
              "error"
            );
            return;
          }
          // Non-sparse native window failures retain the established direct
          // close fallback below.
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
  // Page creation/deletion has its own rare invalidation lane: canonical-name
  // precedence stays current without listing every page after ordinary saves.
  createEffect(on(pageInventoryRev, () => void refreshPageIdentities(), { defer: true }));

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
          {firstLoadDone() ? "Finishing graph operation…" : "Opening graph storage…"}
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
              <WorkspaceSwitcher />
            </div>
            <Show when={mobileDrawerMode()}>
              <button class="mobile-drawer-close" type="button" aria-label="Close navigation sidebar" onClick={() => dismissDrawerAndRestore("explicit")}>Close</button>
            </Show>
            <Sidebar onActiveNavigationComplete={completeActiveLeftNavigation} />
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
          <Show when={!sidebarOpen()}>
            <WorkspaceSwitcher compact />
          </Show>
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
            <CalendarJump triggerClass="topbar-optional-action" onOpenReady={(open) => { openCalendarJump = open; }} />
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
            <TopbarOverflowMenu
              onCalendar={topbarActions.calendar}
              onJournals={topbarActions.journals}
              onToggleTheme={topbarActions.theme}
              onToggleRightSidebar={topbarActions.rightSidebar}
              onBack={topbarActions.back}
              onForward={topbarActions.forward}
              canGoBack={canGoBack}
              canGoForward={canGoForward}
            />
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
        {/* Direct Files conflicts are Concord objects rendered in-page. The old
            global two-button surface remains only for actor-owned managed
            conflicts until that protocol adopts the multi-side queue. */}
        <Show when={conflicts().some((name) => conflictObservationKindFor(name) === "managed")}>
          <ConflictBar />
        </Show>
        <InPageFind />
        </DrawerBackground>
        {/* Everything below the topbar lives in this row, so the topbar (and its
            window controls at the far right) spans the full window width and the
            right sidebar / PDF pane sit UNDER it — not beside the close button. */}
        <div class="content-row">
          <DrawerBackground class="drawer-workspace" blockedBy="right">
          <PaneEdgeHighlights />
          <PaneSelectHint />
          <PaneTree node={visibleLayoutNode()} path={[]} />
          </DrawerBackground>
          <RightSidebar />
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
      <QuickSwitcher />
      <ContextMenu />
      <DatePicker />
      <FormulaEditor />
      <DrawerBackground class="drawer-floating-background" blockedBy="any">
        <MobileKeyboardToolbar />
      </DrawerBackground>
      <PageProps />
      <ExportModal />
      <PdfExportDialog />
      <Show when={settingsOpen()}>
        <Suspense>
          <Settings />
        </Suspense>
      </Show>
      <HelpPopup />
      {/* First-run onboarding: covers the (empty) app when no graph is configured.
          Rendered before Toasts so a "couldn't create graph" toast still shows on top. */}
      <WelcomeLayer
        mandatory={(globalThis as any).__FORCE_WELCOME__ === true || (firstLoadDone() && !graphMeta())}
        optionalOpen={welcomeOpen()}
        onClose={closeWelcome}
      />
      <StartupRecoveryLayer controller={startupRecovery} />
      <Show when={freshnessVisible()}>
        <div class="focus-freshness-barrier" role="status" aria-live="polite">
          Refreshing changes from disk…
        </div>
      </Show>
      <DrawerBackground class="drawer-floating-background" blockedBy="any">
        <AbsenceSweepCenter />
        <Toasts />
      </DrawerBackground>
      <Lightbox />
      <AudioOverlay />
    </div>
  );
}
