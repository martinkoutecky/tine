import { For, Show, type Accessor, type JSX, type Setter } from "solid-js";
import { isMac } from "../nativeChrome";
import { openPage } from "../router";
import { hlsPageName } from "../pdf";
import type { PdfOutlineItem } from "./pdfOutline";
import { PdfOutlineTree } from "./PdfOutlineTree";
import { COLORS, COLOR_RGBA, PDF_THEMES, type PdfTheme } from "./pdfViewerPalette";

/** Presentation-only reader view. All document, graph, and persistence work is
 * owned by PdfViewer; these signals and actions only render controls and relay
 * user events. Cost O(visible controls), with no graph I/O in this module. */
export interface PdfViewerViewModel {
  props: { filename: string; label: string; navigation?: () => { highlightId?: string } | null;
    onOpenNotes?: (block?: string) => void };
  theme: Accessor<PdfTheme>;
  ready: Accessor<boolean>;
  unsavedHighlights: Accessor<boolean>;
  highlightCleanupPending: Accessor<boolean>;
  curPage: Accessor<number>;
  pageField: Accessor<string>;
  numPages: Accessor<number>;
  findOpen: Accessor<boolean>;
  scale: Accessor<number>;
  areaMode: Accessor<boolean>;
  outlineOpen: Accessor<boolean>;
  settingsOpen: Accessor<boolean>;
  highlightConflict: Accessor<unknown>;
  highlightDecisionBusy: Accessor<boolean>;
  outlineReady: Accessor<boolean>;
  outlineTruncated: Accessor<boolean>;
  outlineItems: Accessor<PdfOutlineItem[]>;
  expandedOutlineIds: Accessor<Set<string>>;
  findQuery: Accessor<string>;
  findCount: Accessor<number>;
  findCur: Accessor<number>;
  findTruncated: Accessor<boolean>;
  loadError: Accessor<string | null>;
  menu: Accessor<{ x: number; y: number; id?: string } | null>;
  pendingArea: () => boolean;
  setPageInputFocused: (focused: boolean) => void;
  setPageField: (value: string) => void;
  setScale: (value: number) => void;
  setAreaMode: Setter<boolean>;
  setOutlineOpen: Setter<boolean>;
  setSettingsOpen: Setter<boolean>;
  scrollToPage: (page: number) => void;
  commitPageField: () => void;
  openFind: () => void;
  closeFind: () => void;
  zoomBy: (factor: number) => void;
  fitWidthScale: () => number;
  fitHeightScale: () => number;
  closeSafely: () => Promise<void>;
  keepMineHighlights: () => Promise<unknown>;
  useDiskHighlights: () => Promise<unknown>;
  discardMineHighlights: () => Promise<unknown>;
  retryHighlightCleanup: () => Promise<unknown>;
  chooseTheme: (theme: PdfTheme) => void;
  toggleOutlineItem: (id: string) => void;
  activateOutlineItem: (item: PdfOutlineItem) => Promise<void>;
  scheduleFind: (query: string) => void;
  nextMatch: (delta: number) => void;
  onAreaDown: (event: MouseEvent) => void;
  onMouseUp: (event: MouseEvent) => void;
  onTouchSelectionEnd: (event: TouchEvent) => void;
  onWheel: (event: WheelEvent) => void;
  onScroll: () => void;
  recolorHighlight: (id: string, color: string) => Promise<void>;
  createAreaHighlight: (color: string) => Promise<void>;
  createHighlight: (color: string) => Promise<void>;
  copyExistingHighlightRef: (id: string) => Promise<void>;
  openExistingHighlightReferences: (id: string) => Promise<void>;
  deleteHighlight: (id: string) => Promise<void>;
  setViewerRootEl: (el: HTMLDivElement) => void;
  setFindTriggerEl: (el: HTMLButtonElement) => void;
  setSettingsTriggerEl: (el: HTMLButtonElement) => void;
  setOutlineTriggerEl: (el: HTMLButtonElement) => void;
  setSettingsRootEl: (el: HTMLDivElement) => void;
  setOutlineRootEl: (el: HTMLDivElement) => void;
  setFindRootEl: (el: HTMLDivElement) => void;
  setFindInputEl: (el: HTMLInputElement) => void;
  setScrollRef: (el: HTMLDivElement) => void;
  setHighlightMenuRootEl: (el: HTMLDivElement) => void;
}

/** Render the reader controls and overlays supplied by one mounted viewer. */
export function PdfViewerView(view: PdfViewerViewModel): JSX.Element {
  const { props, theme, ready, unsavedHighlights, highlightCleanupPending,
    curPage, pageField, numPages, findOpen, scale, areaMode, outlineOpen,
    settingsOpen, highlightConflict, highlightDecisionBusy, outlineReady,
    outlineTruncated, outlineItems, expandedOutlineIds, findQuery, findCount,
    findCur, findTruncated, loadError, menu, pendingArea, setPageInputFocused,
    setPageField, setScale, setAreaMode, setOutlineOpen, setSettingsOpen,
    scrollToPage, commitPageField, openFind, closeFind, zoomBy, fitWidthScale,
    fitHeightScale, closeSafely, keepMineHighlights, useDiskHighlights,
    discardMineHighlights, retryHighlightCleanup, chooseTheme, toggleOutlineItem,
    activateOutlineItem, scheduleFind, nextMatch, onAreaDown, onMouseUp,
    onTouchSelectionEnd, onWheel, onScroll, recolorHighlight, createAreaHighlight,
    createHighlight, copyExistingHighlightRef, openExistingHighlightReferences,
    deleteHighlight, setViewerRootEl, setFindTriggerEl, setSettingsTriggerEl,
    setOutlineTriggerEl, setSettingsRootEl, setOutlineRootEl, setFindRootEl,
    setFindInputEl, setScrollRef, setHighlightMenuRootEl } = view;
  return (
    <div
      ref={setViewerRootEl}
      class="pdf-viewer"
      data-theme={theme()}
      data-pdf-filename={props.filename}
      data-pdf-highlight-target={props.navigation?.()?.highlightId}
      data-pdf-ready={ready() ? "true" : "false"}
      data-pdf-highlights-unsaved={unsavedHighlights() ? "true" : "false"}
    >
      <div class="pdf-toolbar">
        <span class="pdf-title">{props.label}</span>
        <Show when={unsavedHighlights()}><span class="pdf-unsaved">{highlightCleanupPending() ? "Area image cleanup pending" : "Unsaved highlight"}</span></Show>
        <div class="pdf-toolbar-actions">
          <div class="pdf-pager">
            <button class="icon-btn" title="Previous page" onClick={() => scrollToPage(curPage() - 1)}>
              ‹
            </button>
            <input
              class="pdf-page-input"
              title="Page — type a number and press Enter to jump"
              value={pageField()}
              onFocus={(e) => {
                setPageInputFocused(true);
                e.currentTarget.select();
              }}
              onInput={(e) => setPageField(e.currentTarget.value)}
              onBlur={() => {
                setPageInputFocused(false);
                commitPageField();
                setPageField(String(curPage()));
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  commitPageField();
                  e.currentTarget.blur();
                }
              }}
            />
            <span class="pdf-page-total">/ {numPages()}</span>
            <button class="icon-btn" title="Next page" onClick={() => scrollToPage(curPage() + 1)}>
              ›
            </button>
          </div>
          <button
            ref={setFindTriggerEl}
            class="icon-btn"
            classList={{ active: findOpen() }}
            title="Find in document (Ctrl+F)"
            onClick={() => (findOpen() ? closeFind() : openFind())}
          >
            🔍
          </button>
          <div class="pdf-zoom">
            <button class="icon-btn" title="Zoom out" onClick={() => zoomBy(1 / 1.1)}>
              −
            </button>
            <span class="pdf-zoom-level">{Math.round(scale() * 100)}%</span>
            <button class="icon-btn" title="Zoom in" onClick={() => zoomBy(1.1)}>
              +
            </button>
            <button class="icon-btn pdf-overflow-action" title="Fit width" onClick={() => setScale(fitWidthScale())}>
              ↔
            </button>
            <button class="icon-btn pdf-overflow-action" title="Fit height" onClick={() => setScale(fitHeightScale())}>
              ↕
            </button>
          </div>
          <button
            class="icon-btn pdf-overflow-action"
            classList={{ active: areaMode() }}
            title={`Area highlight (${isMac ? "⌘" : "Shift"}) — drag a rectangle to capture a region as an image`}
            onClick={() => setAreaMode((v) => !v)}
          >
            ▭
          </button>
          <button
            class="pdf-notes-btn pdf-overflow-action"
            title="Open highlights & notes page"
            onClick={() => props.onOpenNotes ? props.onOpenNotes() : openPage(hlsPageName(props.filename), "page")}
          >
            Notes
          </button>
          <button
            ref={setOutlineTriggerEl}
            type="button"
            class="icon-btn pdf-overflow-action"
            classList={{ active: outlineOpen() }}
            title="Outline"
            aria-label="Outline"
            aria-expanded={outlineOpen()}
            onClick={() => {
              setSettingsOpen(false);
              setOutlineOpen((open) => !open);
            }}
          >
            ☷
          </button>
          <button
            ref={setSettingsTriggerEl}
            type="button"
            class="icon-btn"
            classList={{ active: settingsOpen() }}
            title="More settings"
            aria-label="More settings"
            aria-expanded={settingsOpen()}
            onClick={() => {
              setOutlineOpen(false);
              setSettingsOpen((open) => !open);
            }}
          >
            ⋯
          </button>
          <button class="icon-btn pdf-close-btn" title="Close PDF" aria-label="Close PDF" onClick={() => void closeSafely()}>
            ✕
          </button>
        </div>
      </div>
      <Show when={highlightConflict()}>
        <div class="conflict-banner pdf-highlight-conflict" role="alert">
          <span class="conflict-msg">Highlight conflict. Resolve it before editing more highlights.</span>
          <div class="conflict-actions">
            <button class="conflict-btn keep" disabled={highlightDecisionBusy()} onClick={() => void keepMineHighlights()}>Keep mine</button>
            <button class="conflict-btn" disabled={highlightDecisionBusy()} onClick={() => void useDiskHighlights()}>Use disk version</button>
            <button class="conflict-btn" disabled={highlightDecisionBusy()} onClick={() => void discardMineHighlights()}>Discard my changes</button>
          </div>
        </div>
      </Show>
      <Show when={highlightCleanupPending()}>
        <div class="conflict-banner pdf-highlight-cleanup" role="alert">
          <span class="conflict-msg">Area image cleanup is pending. Retry before closing or switching graphs.</span>
          <button class="conflict-btn" disabled={highlightDecisionBusy()} onClick={() => void retryHighlightCleanup()}>Retry cleanup</button>
        </div>
      </Show>
      <Show when={settingsOpen()}>
        <div ref={setSettingsRootEl} class="pdf-settings-menu" role="dialog" aria-label="PDF settings">
          <div class="pdf-settings-overflow" aria-label="Reader tools">
            <button type="button" onClick={() => { setScale(fitWidthScale()); setSettingsOpen(false); }}>Fit width</button>
            <button type="button" onClick={() => { setScale(fitHeightScale()); setSettingsOpen(false); }}>Fit height</button>
            <button
              type="button"
              aria-pressed={areaMode()}
              onClick={() => { setAreaMode((v) => !v); setSettingsOpen(false); }}
            >
              Area highlight
            </button>
            <button type="button" onClick={() => { if (props.onOpenNotes) props.onOpenNotes(); else openPage(hlsPageName(props.filename), "page"); setSettingsOpen(false); }}>Notes</button>
            <button type="button" onClick={() => { setSettingsOpen(false); setOutlineOpen(true); }}>Outline</button>
          </div>
          <div class="pdf-settings-heading">Theme</div>
          <div class="pdf-theme-choices" role="group" aria-label="PDF theme">
            <For each={PDF_THEMES}>
              {(choice) => {
                const label = `${choice[0].toUpperCase()}${choice.slice(1)}`;
                return (
                  <button
                    type="button"
                    class="pdf-theme-choice"
                    classList={{ active: theme() === choice }}
                    aria-label={`${label} PDF theme`}
                    aria-pressed={theme() === choice}
                    onClick={() => chooseTheme(choice)}
                  >
                    {label}
                  </button>
                );
              }}
            </For>
          </div>
        </div>
      </Show>
      <Show when={outlineOpen()}>
        <div ref={setOutlineRootEl} class="pdf-outline-panel" role="dialog" aria-label="Document outline">
          <div class="pdf-outline-heading">Outline</div>
          <Show when={outlineReady()} fallback={<div class="pdf-outline-loading">Loading outline…</div>}>
            <Show when={outlineTruncated()}>
              <div class="pdf-outline-truncated" role="status">Outline too large; partially shown.</div>
            </Show>
            <Show when={outlineItems().length} fallback={<Show when={!outlineTruncated()}><div class="pdf-outline-empty">No outlines</div></Show>}>
              <PdfOutlineTree
                items={outlineItems()}
                expanded={(id) => expandedOutlineIds().has(id)}
                toggle={toggleOutlineItem}
                activate={(item) => void activateOutlineItem(item)}
              />
            </Show>
          </Show>
        </div>
      </Show>
      <Show when={findOpen()}>
        <div ref={setFindRootEl} class="pdf-find-bar">
          <input
            ref={setFindInputEl}
            class="pdf-find-input"
            placeholder="Find in document"
            value={findQuery()}
            onInput={(e) => scheduleFind(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                nextMatch(e.shiftKey ? -1 : 1);
              }
            }}
          />
          <span class="pdf-find-count">
            {findCount()
              ? `${findCur()} / ${findCount()}${findTruncated() ? "+" : ""}`
              : findQuery().trim()
                ? findTruncated() ? "No results in scanned text+" : "No results"
                : ""}
          </span>
          <button class="icon-btn" title="Previous match (Shift+Enter)" onClick={() => nextMatch(-1)}>
            ↑
          </button>
          <button class="icon-btn" title="Next match (Enter)" onClick={() => nextMatch(1)}>
            ↓
          </button>
          <button class="icon-btn" title="Close (Esc)" onClick={closeFind}>
            ✕
          </button>
        </div>
      </Show>
      <Show
        when={!loadError()}
        fallback={<div class="pdf-load-error">Couldn't open this PDF: <code>{loadError()}</code></div>}
      >
        <div
          class="pdf-scroll"
          classList={{ "area-mode": areaMode() }}
          ref={setScrollRef}
          onMouseDown={onAreaDown}
          onMouseUp={onMouseUp}
          onTouchEnd={onTouchSelectionEnd}
          onWheel={onWheel}
          onScroll={onScroll}
        />
      </Show>
      <Show when={menu()}>
        <div
          ref={setHighlightMenuRootEl}
          class="pdf-color-menu"
          style={{ left: `${menu()!.x}px`, top: `${menu()!.y + 8}px` }}
        >
          <For each={COLORS}>
            {(c) => (
              <button
                class="pdf-color-swatch"
                style={{ background: COLOR_RGBA[c] }}
                onPointerDown={(e) => {
                  e.preventDefault();
                  const m = menu()!;
                  if (m.id) void recolorHighlight(m.id, c); // recolor existing
                  else if (pendingArea()) void createAreaHighlight(c); // create area after explicit color choice
                  else void createHighlight(c); // create new
                }}
              />
            )}
          </For>
          <Show when={menu()!.id}>
            <button
              class="pdf-hl-action"
              onPointerDown={(e) => {
                e.preventDefault();
                void copyExistingHighlightRef(menu()!.id!);
              }}
            >
              Copy ref
            </button>
            <button
              class="pdf-hl-action"
              onPointerDown={(e) => {
                e.preventDefault();
                void openExistingHighlightReferences(menu()!.id!);
              }}
            >
              Linked references
            </button>
          </Show>
          <Show when={menu()!.id}>
            <button
              class="pdf-hl-remove"
              title="Remove highlight"
              onPointerDown={(e) => {
                e.preventDefault();
                void deleteHighlight(menu()!.id!);
              }}
            >
              ✕
            </button>
          </Show>
        </div>
      </Show>
    </div>
  );
}
