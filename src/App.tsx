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
import { CalendarJump } from "./components/CalendarJump";
import { ConflictBar } from "./components/ConflictBar";
import { RightSidebar } from "./components/RightSidebar";
import { Settings } from "./components/Settings";
import { DatePicker } from "./components/DatePicker";
import { installKeybindings } from "./keybindings";
import { loadGraphPath, persistedGraphPath, refreshAliases } from "./graph";
import { goBack, goForward, canGoBack, canGoForward } from "./router";
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
  openSettings,
  shortcutOverrides,
  wideMode,
  documentMode,
  focusMode,
  dimInactiveBlocks,
  exitFocusMode,
  dataRev,
} from "./ui";
import { editingId, flushAll } from "./store";
import { isTauri } from "./backend";

export function App(): JSX.Element {
  onMount(async () => {
    const graphPath = persistedGraphPath() || ((window as any).__GRAPH_PATH__ ?? "");
    await loadGraphPath(graphPath);
  });

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
        closing = true;
        e.preventDefault();
        try {
          // Cap the wait so a stuck save IPC can't prevent quitting.
          await Promise.race([flushAll(), new Promise((r) => setTimeout(r, 1500))]);
        } catch {
          // never block quitting on a save error
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

  // After edits settle (dataRev bumps), refresh the alias map so changing an
  // alias:: doesn't leave navigation resolving to the old canonical page. The
  // Rust side caches aliases, so this is cheap unless a save actually changed them.
  createEffect(on(dataRev, () => void refreshAliases(), { defer: true }));

  // (Re)install keybindings whenever config or the user's local overrides change
  // (precedence: defaults < config.edn :shortcuts < Settings overrides).
  createEffect(() => {
    const cfg = graphMeta()?.shortcuts ?? {};
    const merged = { ...cfg, ...shortcutOverrides() };
    const dispose = installKeybindings(merged);
    onCleanup(dispose);
  });

  return (
    <div
      class="app-container"
      classList={{
        "sidebar-collapsed": !sidebarOpen(),
        "wide-mode": wideMode(),
        "document-mode": documentMode(),
        "focus-mode": focusMode(),
        // Dim only kicks in while a block is being edited — otherwise the whole
        // page would sit faded. This gives the typewriter "spotlight the line".
        "dim-mode": dimInactiveBlocks() && !!editingId(),
      }}
    >
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
        <header class="topbar">
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
          <TabBar />
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
            <button class="icon-btn" title="Settings (t s)" onClick={openSettings}>
              <svg viewBox="0 0 24 24" class="nav-icon" aria-hidden="true">
                <path
                  fill="currentColor"
                  d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 00.12-.61l-1.92-3.32a.488.488 0 00-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 00-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 00-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.07.62-.07.94s.02.64.07.94l-2.03 1.58a.49.49 0 00-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 00-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"
                />
              </svg>
            </button>
          </div>
        </header>
        <ConflictBar />
        <main class="main-content">
          <div class="main-content-inner">
            <PageView />
          </div>
        </main>
      </div>
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
      <Settings />
      <Toasts />
      <Lightbox />
    </div>
  );
}
