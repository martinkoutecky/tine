import { Show, createEffect, onCleanup, onMount, type JSX } from "solid-js";
import { Sidebar } from "./components/Sidebar";
import { PageView } from "./components/Page";
import { QuickSwitcher } from "./components/QuickSwitcher";
import { PdfViewer } from "./components/PdfViewer";
import { TabBar } from "./components/TabBar";
import { ContextMenu } from "./components/ContextMenu";
import { CalendarJump } from "./components/CalendarJump";
import { ConflictBar } from "./components/ConflictBar";
import { RightSidebar } from "./components/RightSidebar";
import { Settings } from "./components/Settings";
import { DatePicker } from "./components/DatePicker";
import { installKeybindings } from "./keybindings";
import { loadGraphPath, persistedGraphPath } from "./graph";
import { goBack, goForward, canGoBack, canGoForward } from "./router";
import {
  theme,
  toggleTheme,
  sidebarOpen,
  toggleSidebar,
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
} from "./ui";

export function App(): JSX.Element {
  onMount(async () => {
    const graphPath = persistedGraphPath() || ((window as any).__GRAPH_PATH__ ?? "");
    await loadGraphPath(graphPath);
  });

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
            <button class="icon-btn" title="Settings" onClick={openSettings}>
              <svg viewBox="0 0 24 24" class="nav-icon">
                <circle cx="12" cy="12" r="3" fill="none" stroke="currentColor" stroke-width="1.6" />
                <path
                  d="M12 3v2m0 14v2m9-9h-2M5 12H3m13.5-6.5l-1.5 1.5m-6 6l-1.5 1.5m9 0l-1.5-1.5m-6-6L6.5 5.5"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.4"
                />
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
          <PdfViewer
            filename={pdfTarget()!.filename}
            label={pdfTarget()!.label}
            page={pdfTarget()!.page}
          />
        </div>
      </Show>
      <QuickSwitcher />
      <ContextMenu />
      <DatePicker />
      <Settings />
    </div>
  );
}
