import { Show, Suspense, createEffect, lazy, on, onCleanup, onMount, type JSX } from "solid-js";
import { Sidebar } from "./components/Sidebar";
import { PageView } from "./components/Page";
import { QuickSwitcher } from "./components/QuickSwitcher";
// pdf.js (~hundreds of KB) is heavy and most sessions never open a PDF — load
// the viewer only when one is opened.
const PdfViewer = lazy(() =>
  import("./components/PdfViewer").then((m) => ({ default: m.PdfViewer }))
);
import { TabBar } from "./components/TabBar";
import { ContextMenu } from "./components/ContextMenu";
import { Toasts, Lightbox } from "./components/Toasts";
import { AudioOverlay } from "./components/AudioOverlay";
import { CalendarJump } from "./components/CalendarJump";
import { ConflictBar } from "./components/ConflictBar";
import { RightSidebar } from "./components/RightSidebar";
import { Settings } from "./components/Settings";
import { HelpPopup } from "./components/HelpShortcuts";
import { DatePicker } from "./components/DatePicker";
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
import { goBack, goForward, canGoBack, canGoForward, flushSession } from "./router";
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
  installPaneTracker,
  pushToast,
  refreshSyncConflicts,
} from "./ui";
import { applyZoom, installInterfaceZoomKeys, installInterfaceZoomWheel } from "./zoom";
import { flushAll, appendToTodayJournal, captureToPage } from "./store";
import type { QuickCaptureAck, QuickCaptureRequest } from "./quickCaptureAck";
import { backend, isTauri } from "./backend";
import { parserFailed } from "./render/parse";
import { warnIfSoftwareRendering } from "./gpu";
import { initSmoothScroll } from "./smoothScroll";
import { initCopySettings } from "./copySettings";
import { initNavSettings } from "./navSettings";
import { initLocalFileSettings } from "./localFileSettings";
import { initAssetSettings } from "./assetSettings";
import { initSpellcheckSettings } from "./spellcheckSettings";
import { initLinkDefault } from "./editor/linkDefault";
import { initDebug, dbg } from "./debug";
import { WindowControls, ResizeGrips, installWindowChrome, maximized } from "./components/WindowChrome";
import { initNativeChrome, isMac, isMobilePlatform, osDrawsWindowControls } from "./nativeChrome";

export function App(): JSX.Element {
  // Startup debug trace (TINE_DEBUG=1 / --debug): forward UI milestones + errors
  // into the backend log so a remote "bad startup" is diagnosable in one file.
  onMount(() => void initDebug());

  onMount(async () => {
    const graphPath = persistedGraphPath() || ((window as any).__GRAPH_PATH__ ?? "");
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
  // Load the asset-filename format template (Settings → Backups → Asset names).
  onMount(() => void initAssetSettings());
  // Load spellcheck prefs (toggle + languages) and apply them to the webview.
  onMount(() => void initSpellcheckSettings());

  // Load the `[[`/`#` autocomplete default-action preference (link-first vs create).
  onMount(() => void initLinkDefault());

  // Persist pending edits before the window closes — the 400ms save debounce
  // would otherwise drop the last keystrokes typed right before quitting.
  // Hardened so it can NEVER wedge the window open: a re-entry guard, a timeout
  // cap on the flush, and a destroy()→close() fallback.
  onMount(() => {
    if (!isTauri()) return;
    let unlisten = () => {};
    let closing = false;
    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const w = getCurrentWindow();
      unlisten = await w.onCloseRequested(async (e) => {
        if (closing) return; // second pass (from close() below) — let it through
        e.preventDefault();
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
          const quit = await backend().confirm(
            "Tine has unsaved changes that couldn't be saved (a conflict or a stuck save).\n\nQuit anyway and lose them?",
            "Unsaved changes"
          );
          if (!quit) return; // stay open
        }
        // Persist the final tab session too — the 150ms debounce may not have
        // fired if the last tab action came right before quitting. Capped so a
        // stuck IPC can't wedge the window open.
        try {
          await Promise.race([flushSession(), new Promise((r) => setTimeout(r, 1000))]);
        } catch {
          // best-effort
        }
        closing = true;
        // Quit via the backend so it can SIGKILL WebKitGTK's helper processes
        // before they run their crash-y GL-driver exit teardown (GH #28). This
        // never resolves (the process exits); the destroy()/close() below are only
        // reached if the quit IPC didn't take (older backend, non-Linux edge), so
        // the window still closes.
        try {
          await Promise.race([
            backend().quit(),
            new Promise((r) => setTimeout(r, 2000)),
          ]);
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
  // `quick-capture` event that the MAIN window turns into an append to today's
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
      const { emit, listen } = await import("@tauri-apps/api/event");
      const ack = (id: string | undefined, ok: boolean) => {
        if (id) void emit("quick-capture-ack", { id, ok } satisfies QuickCaptureAck);
      };
      unlisten = await listen<QuickCaptureRequest>("quick-capture", async (e) => {
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
      const { emit, listen } = await import("@tauri-apps/api/event");
      unlisten = await listen("capture-request-theme", () => {
        void emit("capture-apply-theme", { theme: theme() });
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
      const { emit } = await import("@tauri-apps/api/event");
      await emit("capture-apply-theme", { theme: t });
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
      const { emit } = await import("@tauri-apps/api/event");
      await emit("capture-apply-shortcuts", latestShortcuts);
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
      const { listen } = await import("@tauri-apps/api/event");
      unlisten = await listen("capture-request-shortcuts", broadcastShortcuts);
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
              title="Go back"
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
          <Show when={!isMobilePlatform} fallback={<div class="topbar-spacer" data-tauri-drag-region />}>
            <TabBar />
          </Show>
          <div class="topbar-right">
            <CalendarJump />
            <button class="icon-btn" title="Search (Ctrl+K)" onClick={openSwitcher}>
              <svg viewBox="0 0 24 24" class="nav-icon">
                <circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" stroke-width="1.7" />
                <line x1="16.5" y1="16.5" x2="21" y2="21" stroke="currentColor" stroke-width="1.7" />
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
          <main class="main-content">
            <div class="main-content-inner">
              <PageView />
            </div>
          </main>
          <RightSidebar />
          <Show when={pdfTarget()}>
        <div class="pdf-pane" style={{ flex: `0 0 ${pdfPaneWidth()}px`, width: `${pdfPaneWidth()}px` }}>
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
            <PdfViewer
              filename={pdfTarget()!.filename}
              label={pdfTarget()!.label}
              page={pdfTarget()!.page}
            />
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
