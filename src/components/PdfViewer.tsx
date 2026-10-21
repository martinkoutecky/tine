import { shortcutKey } from "../shortcutKey";
import { msg } from "../i18n";
import { createEffect, createMemo, createSignal, createUniqueId, on, onCleanup, onMount, untrack, type JSX } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import { sanitizeOutlineItems, type PdfOutlineItem } from "./pdfOutline";
import { PdfViewerView } from "./pdfViewerView";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { backend } from "../backend";
import { captureBinding } from "../binding";
import { bindingOwner, graphOwner, latestOwner, readOwned, writeOwned } from "../owned";
import { errorFamily } from "../errorFamily";
import { writeClipboardText } from "../clipboard";
import { activePane, requestBlockReferences } from "../ui";
import { focusedRouter } from "../panes";
import { pushToast } from "../toasts";
import { trackAssetWrite } from "../document";
import { openPageAtBlock } from "../router";
import { areaHighlightPosition, hlsPageName, rectInPageSpace, rectWithSourceSpace, type PdfPageDimensions } from "../pdf";
import { decideWheelZoomGesture, type WheelZoomGestureState } from "../zoom";
import type { Highlight, Rect } from "../types";
import { isMac } from "../nativeChrome";
import { touchCapable } from "../inputCapability";
import { pdfTakesOverPane } from "../viewportClass";
import { dismissOnOutsidePointer, registerTransientLayer } from "../transientLayers";
import { createPdfHighlightState } from "./pdfHighlightState";
import { pdfTextSelection, type PdfTextSelection } from "./pdfSelection";
import { createPdfFind } from "./pdfFind";
import { createPdfTiles, PDF_TILE_PIXEL_BUDGET } from "./pdfTiles";
import { createWheelZoomAccumulator, highestPriorityPage, PDF_PREFETCH_PAGES, PDF_ZOOM_SETTLE_MS } from "./pdfRenderQueue";
import { COLOR_RGB, COLOR_RGBA, type PdfTheme } from "./pdfViewerPalette";
export { PDF_FIND_TEXT_CACHE_BYTES, PDF_FIND_PAGE_TEXT_BYTES, PDF_FIND_MATCH_CAP } from "./pdfFind";
import {
  isPdfOwnershipCurrent,
  drainPdfWork,
  registerPdfParticipant,
  trackPdfMutation,
  type PdfOwnership,
} from "../pdfOwnership";
import { PDF_THEME_KEY, MAX_PDF_BYTES, MAX_PDF_PAGES, PDF_CANVAS_CACHE_PIXEL_BUDGET,
  PDF_CANVAS_CACHE_PAGE_CAP, storedPdfTheme, isPdfPageRef, discardPdfDocument,
  isPdfAreaModifier, pageDimensionsError, safeCanvasSize, cropPdfArea,
  errorMessage, type PendingArea, type PdfTarget } from "./pdfViewerPrimitives";
import { newIntersectionObserver, requestFrame, useOwnerWindow } from "../windowRealm";
pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

export { PDF_CANVAS_CACHE_PIXEL_BUDGET, isPdfAreaModifier } from "./pdfViewerPrimitives";
export { KeyedPdfViewer } from "./KeyedPdfViewer";

/** Render the active PDF and its highlights through the owned document paths.
 * Opening reads sidecar state, asset listing and up to 256 MiB of PDF data;
 * reading reports position to the pane session without graph writes. Annotation failures toast;
 * failed highlight saves stay marked and block drain until resolved. */
export function PdfViewer(props: {
  filename: string;
  label: string;
  owner: PdfOwnership;
  page?: number;
  scale?: number;
  navigation?: () => PdfTarget | null;
  navigationKey?: () => string;
  focused?: () => boolean;
  onClose?: () => void;
  onOpenNotes?: (block?: string) => void;
  onViewState?: (state: { page: number; scale: number }) => void;
}): JSX.Element {
  const owner = props.owner, binding = captureBinding();
  // The Tine window this viewer renders in: its listeners and frames live there.
  const ownerWindow = useOwnerWindow();
  const instanceStem = `pdf-viewer-${createUniqueId()}`;
  const findLayerId = `${instanceStem}-find`;
  const surfaceLayerId = `${instanceStem}-surface`;
  const highlightMenuLayerId = `${instanceStem}-highlight-menu`;
  const settingsLayerId = `${instanceStem}-settings`;
  const outlineLayerId = `${instanceStem}-outline`;
  let viewerRootEl: HTMLDivElement | undefined;
  let scrollRef!: HTMLDivElement;
  let findTriggerEl: HTMLButtonElement | undefined;
  let findRootEl: HTMLDivElement | undefined;
  let highlightMenuRootEl: HTMLDivElement | undefined;
  let settingsTriggerEl: HTMLButtonElement | undefined;
  let settingsRootEl: HTMLDivElement | undefined;
  let outlineTriggerEl: HTMLButtonElement | undefined;
  let outlineRootEl: HTMLDivElement | undefined;
  const pageEls: Record<number, HTMLDivElement> = {};
  const textLayers: Record<number, HTMLDivElement> = {};
  const hlLayers: Record<number, HTMLDivElement> = {};
  // The create-highlight popup (no `id`) OR the edit popup for an existing
  // highlight (`id` set → offers recolor + remove).
  const [menu, setMenu] = createSignal<{ x: number; y: number; id?: string } | null>(null);
  // Area-highlight mode: when on, a drag rubber-bands a rectangle that's cropped
  // from the page canvas into an image highlight (instead of selecting text).
  const [areaMode, setAreaMode] = createSignal(false);
  // Live rubber-band drag state (the page it started on + its element).
  let areaDrag: { page: number; wrap: HTMLElement; startX: number; startY: number; band: HTMLDivElement } | null =
    null;
  const [scale, setScale] = createSignal(1.4);
  const [ready, setReady] = createSignal(false);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  // Page indicator: total pages + the page currently filling the viewport, and a
  // separately-tracked editable field (so a scroll doesn't fight the user typing).
  const [numPages, setNumPages] = createSignal(0);
  const [curPage, setCurPage] = createSignal(1);
  const [pageField, setPageField] = createSignal("1");
  let pageInputFocused = false;
  let scrollRaf: (() => void) | undefined;
  let viewStateReady = false;
  let viewStateBaseline: { page: number; scale: number } | null = null;
  const [theme, setTheme] = createSignal<PdfTheme>(storedPdfTheme());
  const [settingsOpen, setSettingsOpen] = createSignal(false);
  const [outlineOpen, setOutlineOpen] = createSignal(false);
  const [outlineReady, setOutlineReady] = createSignal(false);
  const [outlineItems, setOutlineItems] = createSignal<PdfOutlineItem[]>([]);
  const [outlineTruncated, setOutlineTruncated] = createSignal(false);
  const [expandedOutlineIds, setExpandedOutlineIds] = createSignal<Set<string>>(new Set());
  const viewerRequests = {};
  let pending: PdfTextSelection | null = null;
  let pendingArea: PendingArea | null = null;
  let pdfDoc: pdfjs.PDFDocumentProxy | null = null;
  let disposed = false;
  let activeHighlightId: string | undefined;

  const chooseTheme = (next: PdfTheme) => {
    setTheme(next);
    try {
      window.localStorage.setItem(PDF_THEME_KEY, next);
    } catch {
      // The current mount still changes presentation when storage is unavailable.
    }
  };

  const toggleOutlineItem = (id: string) => {
    setExpandedOutlineIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  async function loadOutline(doc: pdfjs.PDFDocumentProxy) {
    setOutlineReady(false);
    setOutlineItems([]);
    setOutlineTruncated(false);
    setExpandedOutlineIds(new Set<string>());
    let loaded: unknown = [];
    try {
      loaded = await doc.getOutline();
    } catch {
      loaded = [];
    }
    if (disposed || pdfDoc !== doc) return;
    const sanitized = sanitizeOutlineItems(loaded);
    setOutlineItems(sanitized.items);
    setOutlineTruncated(sanitized.truncated);
    setOutlineReady(true);
  }

  // Per-page unscaled dimensions (index 1..N), fetched once so we can size every
  // page wrapper up front — that gives correct scroll geometry without having to
  // rasterize the whole document.
  const dims: { w: number; h: number }[] = [];
  // The scale a page's canvas was last rasterized at (absent = never). Used to
  // skip work and to detect a page that's stale after a zoom.
  const renderedScale: Record<number, number> = {};
  // Live render tasks (so a zoom mid-render can cancel the stale raster).
  const tasks: Record<number, pdfjs.RenderTask> = {};
  const renderGeneration: Record<number, number> = {};
  let layoutGeneration = 0;
  // Pages whose REAL unscaled size has been measured (others use a page-1
  // estimate until first render), so opening a long PDF doesn't parse every page
  // dict before first paint.
  const dimsKnown = new Set<number>();
  // Rendered pages in recency order (LRU). Admission is governed primarily by
  // actual aggregate backing-store pixels, with a page count as a secondary
  // guard. The wrapper stays sized and re-renders on scroll-back.
  const lru: number[] = [];
  const canvasPixels: Record<number, number> = {};
  const tilePages: Record<number, pdfjs.PDFPageProxy> = {};
  const pdfTiles = createPdfTiles((error) => failPdf(errorMessage("Couldn't render this PDF tile", error)));
  // Actual backing-store scale used for each rendered page. This can be lower
  // than devicePixelRatio for an unusually large page, keeping canvas memory
  // bounded while preserving the requested CSS zoom level.
  const renderedPixelRatio: Record<number, number> = {};
  // Pages currently intersecting the viewport. With the prefetch pages ahead of
  // them, they are the only pages the render queue rasterizes.
  const visible = new Set<number>();
  let io: IntersectionObserver | null = null;
  let zoomTimer: number | undefined;
  let zoomSettling = false;
  // Client point a pending scale change keeps fixed (pointer for Ctrl+wheel,
  // top-centre of the view for keys and buttons); consumed by onZoom.
  let zoomOrigin: { x: number; y: number } | null = null;
  // Render queue (pdf.js PDFRenderingQueue): one page paints at a time. Every
  // render's 15 ms chunk asks `mayContinue`; a render that lost priority parks
  // its continuation in `pausedRenders` and resumes when it is highest again.
  let highestPriority: number | null = null;
  let scrollingDown = true;
  let lastScrollTop = 0;
  let pumpQueued = false;
  const inflight: Record<number, { scale: number; promise: Promise<void> }> = {};
  const pausedRenders: Record<number, () => void> = {};
  // Explicit callers (navigation, Find, area crop) awaiting a page: never paused.
  const demanded: Record<number, number> = {};
  // A re-render paints into this detached canvas while the page's previous
  // raster stays on screen (CSS-stretched); it replaces it only when complete.
  const pendingCanvas: Record<number, HTMLCanvasElement> = {};
  // Off-screen pages a prefetch skipped because the canvas budget had no room
  // without evicting a visible or prefetched page, at the scale they were tried.
  const prefetchRefused: Record<number, number> = {};
  // The exact highlight a navigation is still owed a centre-scroll for. It is
  // set when the navigation starts and consumed by applyPendingHighlight once the
  // target page is rasterized at the current scale (its overlay exists and is at
  // the right size). navigateToTarget's own `await renderPage` is not enough: a
  // supersede (IntersectionObserver, zoom settle) makes that call return before
  // the overlay is painted, so whoever finishes the raster applies the scroll.
  let pendingHighlight: { id: string; page: number; current: () => boolean } | null = null;
  // The text layer (hundreds of glyph spans on a math page) is rebuilt OFF the
  // zoom hot path: the canvas sharpens immediately, the text catches up shortly
  // after the view settles. `textScale[n]` is the scale its text was built at;
  // `pendingText` holds pages whose text needs a (re)build.
  const textScale: Record<number, number> = {};
  // The live pdf.js TextLayer instance per page, so a zoom can reposition it
  // cheaply via .update({viewport}) instead of re-extracting text and recreating
  // every glyph span (the expensive work that made zoom-in janky).
  const textLayerObjs: Record<number, any> = {};
  // Owner of a page's text-layer build. Bumped by each build and by freePage, so
  // a pdf.js TextLayer render/update that completes after its page was evicted,
  // re-rastered, re-laid-out or the viewer retired never installs itself
  // (I-20/I-21).
  const textGeneration: Record<number, number> = {};
  // The TextLayer whose render() is still running, so eviction, a newer build or
  // teardown can cancel it instead of letting it finish into a retired page.
  const textInflight: Record<number, { cancel?: () => void }> = {};
  const pendingText = new Set<number>();
  // Scale of the newest text-layer build per page, so a repeat visibility
  // signal does not restart a build already under way for the same raster.
  const textBuildScale: Record<number, number> = {};
  let textTimer: number | undefined;

  async function exactPageDimensions(pageNumber: number): Promise<PdfPageDimensions> {
    if (dimsKnown.has(pageNumber) && dims[pageNumber]) return dims[pageNumber];
    if (!pdfDoc || pageNumber < 1 || pageNumber > pdfDoc.numPages) {
      throw new Error(`highlight refers to missing PDF page ${pageNumber}`);
    }
    const page = await pdfDoc.getPage(pageNumber);
    const viewport = page.getViewport({ scale: 1 });
    const dimensionError = pageDimensionsError(pageNumber, viewport.width, viewport.height);
    if (dimensionError) throw new Error(dimensionError);
    dims[pageNumber] = { w: viewport.width, h: viewport.height };
    dimsKnown.add(pageNumber);
    sizeWrapper(pageNumber);
    return dims[pageNumber];
  }

  async function highlightsForWrite(items: Highlight[]): Promise<Highlight[]> {
    const pages = new Map<number, PdfPageDimensions>();
    for (const highlight of items) {
      const allRects = [highlight.position.bounding, ...highlight.position.rects];
      if (allRects.some((rect) => rect.source_width == null || rect.source_height == null)) {
        pages.set(highlight.page, await exactPageDimensions(highlight.page));
      }
    }
    return items.map((highlight) => {
      const page = pages.get(highlight.page);
      if (!page) return highlight;
      return {
        ...highlight,
        position: {
          ...highlight.position,
          bounding: rectWithSourceSpace(highlight.position.bounding, page),
          rects: highlight.position.rects.map((rect) => rectWithSourceSpace(rect, page)),
        },
      };
    });
  }

  const highlightState = createPdfHighlightState({
    filename: props.filename,
    label: props.label,
    backendGeneration: binding.backendGeneration,
    owner,
    prepare: highlightsForWrite,
  });
  const highlights = highlightState.highlights;
  const unsavedHighlights = highlightState.unsaved;
  const highlightConflict = highlightState.conflict;
  const highlightDecisionBusy = highlightState.decisionBusy;
  const highlightCleanupPending = highlightState.cleanupPending;
  const highlightGraphOwner = bindingOwner(() => isPdfOwnershipCurrent(owner));
  const persist = highlightState.persist;
  const useDiskHighlights = highlightState.useDiskVersion;
  const keepMineHighlights = highlightState.keepMine;
  const discardMineHighlights = highlightState.discardMine;
  const retryHighlightCleanup = highlightState.retryCleanup;

  const copyCreatedHighlightRef = async (id: string) => {
    await writeClipboardText(`((${id}))`);
    pushToast(msg("pdf-copied-highlight-ref"), "success");
  };
  // An OG/externally-created sidecar can outlive or predate its annotation
  // block. Reuse the paired guarded writer before exposing the id: it upserts
  // the hls__ block while preserving notes and refuses conflicts/partial writes.
  const ensureExistingHighlightRef = async (id: string): Promise<boolean> => {
    if (!highlights().some((highlight) => highlight.id === id)) return false;
    return persist();
  };
  const copyExistingHighlightRef = async (id: string) => {
    closeHighlightMenu();
    if (!(await ensureExistingHighlightRef(id))) return;
    await copyCreatedHighlightRef(id);
  };
  const openExistingHighlightReferences = async (id: string) => {
    closeHighlightMenu();
    if (!(await ensureExistingHighlightRef(id))) return;
    requestBlockReferences(id);
    if (props.onOpenNotes) props.onOpenNotes(id);
    else openPageAtBlock(hlsPageName(props.filename), "page", id);
  };
  // Remove a highlight (and its annotation block on the hls page).
  const deleteHighlight = async (id: string) => {
    if (highlightState.editBlocked()) return;
    highlightState.edit(highlights().filter((h) => h.id !== id));
    const intent = highlightState.newIntent();
    closeHighlightMenu();
    await persist(intent);
  };
  const recolorHighlight = async (id: string, color: string) => {
    if (highlightState.editBlocked()) return;
    highlightState.edit(highlights().map((h) => (h.id === id ? { ...h, color } : h)));
    const intent = highlightState.newIntent();
    closeHighlightMenu();
    await persist(intent);
  };

  function closeHighlightMenu() {
    pendingArea = null;
    setMenu(null);
  }
  const clampScale = (s: number) => Math.min(4, Math.max(0.2, s));
  const fitWidthScale = () => (dims[1] ? clampScale((scrollRef.clientWidth - 32) / dims[1].w) : 1);
  const fitHeightScale = () => (dims[1] ? clampScale((scrollRef.clientHeight - 24) / dims[1].h) : 1);

  const scheduleViewState = (page: number, nextScale: number) => {
    if (!isPdfOwnershipCurrent(owner)) return;
    if (!viewStateReady || !Number.isFinite(nextScale) || nextScale <= 0) return;
    if (viewStateBaseline?.page === page && viewStateBaseline?.scale === nextScale) return;
    viewStateBaseline = { page, scale: nextScale };
    props.onViewState?.(viewStateBaseline);
  };

  function failPdf(message: string) {
    if (loadError()) return;
    io?.disconnect();
    io = null;
    clearTimeout(zoomTimer);
    clearTimeout(textTimer);
    findController.cancel();
    releaseAllCanvases();
    pdfTiles.reset();
    for (const k of Object.keys(tasks)) {
      tasks[Number(k)]?.cancel();
      delete tasks[Number(k)];
    }
    scrollRef?.replaceChildren();
    if (pdfDoc) discardPdfDocument(pdfDoc);
    pdfDoc = null;
    setLoadError(message);
  }

  // Build all page wrappers once, sized for the current scale. Cheap: no
  // rasterization — just sized placeholders that the IntersectionObserver fills
  // in as they scroll into view.
  function buildLayout() {
    if (!pdfDoc) return;
    layoutGeneration += 1;
    releaseAllCanvases();
    pdfTiles.reset();
    for (const key of Object.keys(tilePages)) delete tilePages[Number(key)];
    scrollRef.innerHTML = "";
    for (const k of Object.keys(pageEls)) delete pageEls[Number(k)];
    for (const k of Object.keys(textLayers)) delete textLayers[Number(k)];
    for (const k of Object.keys(hlLayers)) delete hlLayers[Number(k)];
    for (const k of Object.keys(renderedScale)) delete renderedScale[Number(k)];
    for (const k of Object.keys(renderedPixelRatio)) delete renderedPixelRatio[Number(k)];
    for (const k of Object.keys(canvasPixels)) delete canvasPixels[Number(k)];
    for (const k of Object.keys(textScale)) delete textScale[Number(k)];
    for (const k of Object.keys(textBuildScale)) delete textBuildScale[Number(k)];
    for (const k of Object.keys(textLayerObjs)) delete textLayerObjs[Number(k)];
    for (const k of Object.keys(textInflight)) cancelInflightText(Number(k));
    for (const k of Object.keys(prefetchRefused)) delete prefetchRefused[Number(k)];
    pendingText.clear();
    clearTimeout(textTimer);
    lru.length = 0;
    visible.clear();
    highestPriority = null;
    io?.disconnect();
    // Exact viewport: what lies ahead is the render queue's explicit prefetch
    // (direction-aware), not an observer margin.
    io = newIntersectionObserver(scrollRef, onIntersect, { root: scrollRef, rootMargin: "0px" });

    // Every wrapper is sized from one container variable (as pdf.js does), so a
    // zoom step relayouts the whole document with a single style write and the
    // stretched canvases (100% of their wrapper) follow without a transform.
    scrollRef.style.setProperty("--scale-factor", String(scale()));
    for (let n = 1; n <= pdfDoc.numPages; n++) {
      const wrap = document.createElement("div");
      wrap.className = "pdf-page";
      wrap.dataset.page = String(n);
      pageEls[n] = wrap;
      sizeWrapper(n);

      const textLayer = document.createElement("div");
      textLayer.className = "textLayer";
      const hl = document.createElement("div");
      hl.className = "pdf-hl-layer";
      wrap.appendChild(textLayer);
      wrap.appendChild(hl);

      scrollRef.appendChild(wrap);
      pageEls[n] = wrap;
      textLayers[n] = textLayer;
      hlLayers[n] = hl;
      io?.observe(wrap);
    }
  }

  function onIntersect(entries: IntersectionObserverEntry[]) {
    for (const e of entries) {
      const n = Number((e.target as HTMLElement).dataset.page);
      if (e.isIntersecting) {
        visible.add(n);
        ensureText(n);
      } else {
        visible.delete(n);
      }
    }
    // The evictable set changed, so a refused prefetch may fit now.
    for (const k of Object.keys(prefetchRefused)) delete prefetchRefused[Number(k)];
    pump();
  }

  // Visible pages, most-visible first (pdf.js getVisibleElements order).
  function visibleByPriority(): number[] {
    const pages = [...visible];
    if (pages.length < 2) return pages;
    const view = scrollRef.getBoundingClientRect();
    const shown = new Map<number, number>();
    for (const n of pages) {
      const r = pageEls[n]?.getBoundingClientRect();
      shown.set(n, r ? Math.max(0, Math.min(r.bottom, view.bottom) - Math.max(r.top, view.top)) : 0);
    }
    return pages.sort((a, b) => (shown.get(b)! - shown.get(a)!) || a - b);
  }

  function schedulePump() {
    if (pumpQueued) return;
    pumpQueued = true;
    queueMicrotask(() => {
      pumpQueued = false;
      pump();
    });
  }

  // Start (or resume) the single highest-priority page render. Runs on
  // visibility changes, once per scroll frame, and whenever a render finishes
  // or pauses. Nothing renders during a zoom gesture: the stretched rasters
  // cover it and the settle pumps once.
  function pump() {
    if (!pdfDoc || disposed || zoomSettling || loadError()) return;
    const s = scale();
    const next = highestPriorityPage({
      visible: visibleByPriority(),
      numPages: pdfDoc.numPages,
      scrollingDown,
      prefetch: PDF_PREFETCH_PAGES,
      done: (page) => renderedScale[page] === s || (!visible.has(page) && prefetchRefused[page] === s),
    });
    highestPriority = next;
    if (next === null) return;
    if (inflight[next]?.scale === s) {
      resumeRender(next);
      return;
    }
    void renderPage(next, false);
  }

  function mayContinue(n: number) {
    return disposed || highestPriority === null || highestPriority === n || (demanded[n] ?? 0) > 0;
  }

  function resumeRender(n: number) {
    const resume = pausedRenders[n];
    if (!resume) return;
    delete pausedRenders[n];
    resume();
  }

  function pageCanvas(n: number): HTMLCanvasElement | null {
    return pageEls[n]?.querySelector(":scope > canvas") ?? null;
  }

  // Rasterize one page at the current scale (no-op if already current). An
  // explicit call (navigation, Find, area crop) is never paused by the queue;
  // a queue call yields to higher-priority pages between 15 ms chunks.
  async function renderPage(n: number, explicit = true): Promise<void> {
    if (!pdfDoc) return;
    const s = scale();
    if (renderedScale[n] === s) {
      if (s > 3 && tilePages[n] && pageEls[n]) pdfTiles.refresh(tilePages[n], n, pageEls[n], scrollRef, s);
      ensureText(n);
      if (pendingHighlight?.page === n) applyPendingHighlight();
      return;
    }
    if (explicit) demanded[n] = (demanded[n] ?? 0) + 1;
    try {
      const running = inflight[n];
      if (running?.scale === s) {
        resumeRender(n);
        return await running.promise;
      }
      const promise = rasterize(n, s);
      inflight[n] = { scale: s, promise };
      try {
        await promise;
      } finally {
        if (inflight[n]?.promise === promise) delete inflight[n];
      }
    } finally {
      if (explicit) {
        demanded[n] -= 1;
        if (demanded[n] <= 0) delete demanded[n];
      }
    }
  }

  function dropPendingCanvas(n: number) {
    const canvas = pendingCanvas[n];
    if (!canvas) return;
    // WebKit may defer freeing a detached canvas's backing store.
    canvas.width = 0;
    canvas.height = 0;
    delete pendingCanvas[n];
  }

  async function rasterize(n: number, s: number) {
    if (!pdfDoc) return;
    const wrap = pageEls[n];
    if (!wrap) return;
    const generation = (renderGeneration[n] ?? 0) + 1;
    const layout = layoutGeneration;
    renderGeneration[n] = generation;
    tasks[n]?.cancel();
    delete tasks[n];
    delete pausedRenders[n];
    dropPendingCanvas(n);

    let page: pdfjs.PDFPageProxy;
    try {
      page = await pdfDoc.getPage(n);
    } catch (err) {
      failPdf(errorMessage("Couldn't render this PDF page", err));
      return;
    }
    if (disposed || layoutGeneration !== layout || renderGeneration[n] !== generation
      || scale() !== s || pageEls[n] !== wrap) return;
    tilePages[n] = page;
    const viewport = page.getViewport({ scale: s });
    // First time we touch this page, learn its real unscaled size and correct the
    // wrapper if the page-1 estimate was off (non-uniform PDF).
    if (!dimsKnown.has(n)) {
      dimsKnown.add(n);
      const rw = viewport.width / s;
      const rh = viewport.height / s;
      const dimensionError = pageDimensionsError(n, rw, rh);
      if (dimensionError) {
        failPdf(dimensionError);
        return;
      }
      if (Math.abs(rw - dims[n].w) > 0.5 || Math.abs(rh - dims[n].h) > 0.5) {
        dims[n] = { w: rw, h: rh };
        sizeWrapper(n);
      }
    }

    // A page that already shows a completed raster keeps it (stretched to the
    // new size) and paints the new one off-screen; a first render paints in
    // place so it appears progressively.
    const shown = pageCanvas(n);
    const swap = !!shown && renderedScale[n] !== undefined;
    const keptPixels = swap ? canvasPixels[n] ?? 0 : 0;
    // Render into a backing store at device-pixel resolution and CSS-size it
    // back down, so text is crisp on HiDPI displays. Cap the device-pixel factor
    // at 2 — beyond that the extra pixels aren't visible but the raster cost (and
    // zoom-in lag) grows quadratically.
    const otherVisiblePixels = [...visible]
      .filter((pageNumber) => pageNumber !== n)
      .reduce((total, pageNumber) => total + (canvasPixels[pageNumber] ?? 0), 0);
    const availablePixels = Math.max(1, PDF_CANVAS_CACHE_PIXEL_BUDGET
      - (s > 3 ? PDF_TILE_PIXEL_BUDGET : 0) - otherVisiblePixels - keptPixels);
    const canvasSize = safeCanvasSize(viewport.width, viewport.height, availablePixels, wrap);
    if (!canvasSize) {
      failPdf(`PDF page ${n} couldn't be sized safely for rendering.`);
      return;
    }
    const nextPixels = canvasSize.width * canvasSize.height;
    const fits = makeRoomForCanvas(n, nextPixels + keptPixels);
    if (!fits && !visible.has(n) && (demanded[n] ?? 0) === 0) {
      // A prefetch never evicts what the reader sees or the other prefetches.
      prefetchRefused[n] = s;
      schedulePump();
      return;
    }
    const canvas = swap ? document.createElement("canvas") : shown ?? document.createElement("canvas");
    if (swap) pendingCanvas[n] = canvas;
    else if (!shown) wrap.insertBefore(canvas, wrap.firstChild);
    const dpr = canvasSize.ratio;
    canvas.width = canvasSize.width;
    canvas.height = canvasSize.height;
    // Reserve immediately, before pdf.js's async render, so concurrent visible
    // page renders see the allocation and cannot all admit the full budget.
    canvasPixels[n] = nextPixels + keptPixels;
    canvas.style.width = "100%";
    canvas.style.height = "100%";

    const task = page.render({
      canvasContext: canvas.getContext("2d")!,
      viewport,
      transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : undefined,
    });
    tasks[n] = task;
    task.onContinue = (cont: () => void) => {
      if (mayContinue(n)) cont();
      else {
        pausedRenders[n] = cont;
        schedulePump();
      }
    };
    const abandon = () => {
      if (pendingCanvas[n] === canvas) {
        dropPendingCanvas(n);
        if (canvasPixels[n] !== undefined) canvasPixels[n] = keptPixels;
      }
    };
    try {
      await task.promise;
    } catch (err) {
      if (tasks[n] === task) delete tasks[n];
      abandon();
      schedulePump();
      if (renderGeneration[n] !== generation || layoutGeneration !== layout) return;
      if ((err as { name?: string } | undefined)?.name === "RenderingCancelledException") return;
      failPdf(errorMessage("Couldn't render this PDF page", err));
      return;
    }
    if (tasks[n] === task) delete tasks[n];
    if (renderGeneration[n] !== generation || layoutGeneration !== layout || scale() !== s) {
      abandon();
      schedulePump();
      return;
    }
    if (swap && shown) {
      shown.replaceWith(canvas);
      shown.width = 0;
      shown.height = 0;
      delete pendingCanvas[n];
    }
    canvasPixels[n] = nextPixels;

    // Canvas is crisp now — the page is usable. Rebuild the (expensive) text
    // layer off the hot path so it doesn't make every zoom step janky.
    renderedScale[n] = s;
    renderedPixelRatio[n] = dpr;
    repaintPage(n);
    if (pendingHighlight?.page === n) applyPendingHighlight();
    scheduleText(n);
    touchLru(n);
    evictCanvases();
    if (s > 3) pdfTiles.refresh(page, n, wrap, scrollRef, s);
    schedulePump();
  }

  function currentNavigation(): PdfTarget {
    const target = props.navigation?.();
    return target?.filename === props.filename
      ? target
      : { filename: props.filename, label: props.label, owner, page: props.page };
  }

  async function navigateToTarget(target: PdfTarget) {
    const current = latestOwner(viewerRequests, "navigation", highlightGraphOwner);
    const highlight = target.highlightId
      ? highlights().find((candidate) => candidate.id === target.highlightId)
      : undefined;
    activeHighlightId = highlight?.id;
    const requestedPage = highlight?.page ?? target.page ?? 1;
    const total = numPages() || pdfDoc?.numPages || 0;
    const inRange = requestedPage >= 1 && (total === 0 || requestedPage <= total);
    let page = requestedPage;
    if (!pageEls[page]) {
      if (!inRange) page = 1;
      else {
        for (let attempt = 0; attempt < 40 && !pageEls[page]; attempt++) {
          await new Promise((resolve) => setTimeout(resolve, 25));
          if (!current() || disposed) return;
        }
        if (!pageEls[page]) page = 1;
      }
    }
    setCurPage(page);
    setPageField(String(page));
    pageEls[page]?.scrollIntoView({ block: "start" });

    if (!highlight) {
      pendingHighlight = null;
      for (const element of scrollRef.querySelectorAll(".pdf-hl-target")) {
        element.classList.remove("pdf-hl-target");
      }
      return;
    }

    // OG carries the highlight entity through open-block-ref! and scrolls the
    // finder to that exact highlight. Render the destination page first so the
    // overlay exists even when it was outside the lazy viewport. The scroll is
    // applied once, by whichever raster of that page completes (see
    // pendingHighlight): this call returns early when another request for the
    // same page supersedes its raster.
    pendingHighlight = { id: highlight.id, page, current };
    for (const element of scrollRef.querySelectorAll(".pdf-hl-target")) {
      element.classList.remove("pdf-hl-target");
    }
    await renderPage(page);
    if (!current()) return;
    applyPendingHighlight();
  }

  // Centre the pending highlight once its page is painted at the current scale
  // and no zoom is settling (a settle would write its page anchor over the
  // scroll). Otherwise leave it pending: the next completed raster of the page
  // (the settle re-renders the visible pages) retries. Runs at most once per
  // navigation; the reader's own scrolling cancels it.
  function applyPendingHighlight() {
    const pending = pendingHighlight;
    if (!pending) return;
    if (!pending.current() || disposed) {
      pendingHighlight = null;
      return;
    }
    if (zoomSettling || renderedScale[pending.page] !== scale()) return;
    const layer = hlLayers[pending.page];
    const exact = layer
      ? Array.from(layer.querySelectorAll<HTMLElement>(".pdf-hl"))
          .find((element) => element.dataset.highlightId === pending.id)
      : undefined;
    if (!exact) return;
    pendingHighlight = null;
    for (const element of scrollRef.querySelectorAll(".pdf-hl-target")) {
      element.classList.remove("pdf-hl-target");
    }
    exact.classList.add("pdf-hl-target");
    exact.scrollIntoView({ block: "center", inline: "nearest" });
  }

  // Record `n` as most-recently rendered.
  function touchLru(n: number) {
    const i = lru.indexOf(n);
    if (i >= 0) lru.splice(i, 1);
    lru.push(n);
  }
  function retainedCanvasPixels(except?: number) {
    return Object.entries(canvasPixels).reduce(
      (total, [page, pixels]) => Number(page) === except ? total : total + pixels,
      0,
    );
  }
  // The prefetch pages the queue currently wants (ahead of the visible span in
  // the scroll direction): eviction spares them as it spares visible pages.
  function prefetchTargets(): Set<number> {
    const targets = new Set<number>();
    const total = pdfDoc?.numPages ?? 0;
    if (!visible.size || !total) return targets;
    const first = Math.min(...visible), last = Math.max(...visible);
    const down = first <= 1 ? true : last >= total ? false : scrollingDown;
    for (let step = 1; step <= PDF_PREFETCH_PAGES; step++) {
      const page = down ? last + step : first - step;
      if (page >= 1 && page <= total) targets.add(page);
    }
    return targets;
  }
  // Free least-recently rendered off-screen pages BEFORE allocating the next
  // backing store. This prevents a valid high-zoom document from transiently
  // building the old count-based 1.5 GiB cache. Visible and prefetch pages are
  // never evicted; returns whether the incoming allocation fits the budget.
  function makeRoomForCanvas(n: number, incomingPixels: number): boolean {
    let total = retainedCanvasPixels(n);
    let count = Object.keys(canvasPixels).filter((page) => Number(page) !== n).length;
    const incomingCount = n >= 1 ? 1 : 0;
    const spared = prefetchTargets();
    const evictable = (page: number) => page !== n && !visible.has(page) && !spared.has(page);
    const over = () => total + incomingPixels > PDF_CANVAS_CACHE_PIXEL_BUDGET
        - (scale() > 3 ? PDF_TILE_PIXEL_BUDGET : 0)
      || count + incomingCount > PDF_CANVAS_CACHE_PAGE_CAP;
    while (over()) {
      // Completed pages use true LRU order. Include an off-screen in-flight
      // allocation as a fallback so rapid scrolling cannot outrun the LRU.
      const candidate = lru.find(evictable)
        ?? Object.keys(canvasPixels).map(Number).find(evictable);
      if (candidate === undefined) return false;
      total -= canvasPixels[candidate] ?? 0;
      count -= canvasPixels[candidate] === undefined ? 0 : 1;
      freePage(candidate);
      const lruIndex = lru.indexOf(candidate);
      if (lruIndex >= 0) lru.splice(lruIndex, 1);
    }
    return true;
  }
  function evictCanvases() {
    makeRoomForCanvas(-1, 0);
  }
  function freePage(n: number) {
    pdfTiles.releasePage(n);
    delete tilePages[n];
    renderGeneration[n] = (renderGeneration[n] ?? 0) + 1;
    tasks[n]?.cancel();
    delete tasks[n];
    delete pausedRenders[n];
    dropPendingCanvas(n);
    const canvas = pageCanvas(n);
    if (canvas) {
      // WebKit may defer freeing a detached canvas's backing store. Resizing to
      // zero releases it synchronously before the DOM node is removed.
      canvas.width = 0;
      canvas.height = 0;
      canvas.remove();
    }
    delete canvasPixels[n];
    delete renderedScale[n];
    delete renderedPixelRatio[n];
    if (textLayers[n]) textLayers[n].innerHTML = "";
    textLayerObjs[n]?.cancel?.();
    cancelInflightText(n);
    delete textLayerObjs[n];
    delete textScale[n];
    delete textBuildScale[n];
    textGeneration[n] = (textGeneration[n] ?? 0) + 1;
    pendingText.delete(n);
  }
  function releaseAllCanvases() {
    for (const page of Object.keys(canvasPixels)) freePage(Number(page));
    lru.length = 0;
  }

  // Coalesced, deferred text-layer (re)build. Runs ~after the view settles, only
  // for visible pages whose text isn't already at the page's current scale.
  function scheduleText(n: number) {
    // FIRST build for a page (scroll-in): do it now, not behind the single shared
    // timer that every other page's render keeps resetting during a scroll — that
    // delay is why pages past the first sometimes had no selectable text layer
    // (no I-beam, so no way to make a regular highlight). Rebuilds (zoom) stay
    // deferred off the hot path.
    if (textScale[n] === undefined) {
      ensureText(n);
      return;
    }
    pendingText.add(n);
    clearTimeout(textTimer);
    // Short: rebuilds follow a re-raster, which the zoom settle already debounced.
    textTimer = window.setTimeout(() => void buildPendingText(), 100);
  }
  async function buildPendingText() {
    const todo = [...pendingText];
    pendingText.clear();
    for (const n of todo) {
      const r = renderedScale[n];
      if (!visible.has(n) || r === undefined || textScale[n] === r) continue;
      await buildTextLayer(n, r);
    }
  }
  // First text-layer build for a rendered page the reader can see. Pages only
  // passed through or prefetched get their text when they become visible.
  function ensureText(n: number) {
    const r = renderedScale[n];
    if (!visible.has(n) || r === undefined || textScale[n] !== undefined || textBuildScale[n] === r) return;
    void buildTextLayer(n, r);
  }
  // A text layer's spans size themselves from --scale-factor. Each layer pins
  // the scale it was laid out at, and a zoom only stretches it with a
  // transform, so a zoom step costs no text relayout (pdf.js cssTransform).
  function pinTextScale(n: number, atScale: number) {
    const layer = textLayers[n];
    if (!layer) return;
    layer.style.setProperty("--scale-factor", String(atScale));
    layer.dataset.scale = String(atScale);
    layer.style.transform = "";
  }
  function fitTextLayer(n: number) {
    const layer = textLayers[n];
    const at = Number(layer?.dataset.scale);
    if (!layer || !at) return;
    layer.style.transform = at === scale() ? "" : `scale(${scale() / at})`;
  }
  function cancelInflightText(n: number) {
    textInflight[n]?.cancel?.();
    delete textInflight[n];
  }
  async function buildTextLayer(n: number, atScale: number) {
    if (!pdfDoc || !textLayers[n]) return;
    cancelInflightText(n);
    const generation = (textGeneration[n] = (textGeneration[n] ?? 0) + 1);
    textBuildScale[n] = atScale;
    const layout = layoutGeneration;
    const container = textLayers[n];
    const buildCurrent = () => !disposed && layoutGeneration === layout && textGeneration[n] === generation
      && renderedScale[n] === atScale && textLayers[n] === container;
    let page: pdfjs.PDFPageProxy;
    try {
      page = await pdfDoc.getPage(n);
    } catch (err) {
      failPdf(errorMessage("Couldn't read this PDF page", err));
      return;
    }
    if (renderedScale[n] !== atScale || !textLayers[n]) return; // re-rastered since
    const viewport = page.getViewport({ scale: atScale });

    // Reposition an existing text layer (cheap) rather than rebuilding it.
    const existing = textLayerObjs[n];
    if (existing) {
      try {
        pinTextScale(n, atScale);
        await existing.update({ viewport });
        if (!buildCurrent()) return;
        textScale[n] = atScale;
        return;
      } catch (err) {
        if (!buildCurrent()) return;
        // pdf.js API mismatch — fall through to a full rebuild.
      }
    }

    let textContent: Awaited<ReturnType<pdfjs.PDFPageProxy["getTextContent"]>>;
    try {
      textContent = await page.getTextContent();
    } catch (err) {
      if (buildCurrent()) failPdf(errorMessage("Couldn't read this PDF text", err));
      return;
    }
    if (!buildCurrent()) return;
    const tl = textLayers[n];
    tl.innerHTML = "";
    pinTextScale(n, atScale);
    const layer = new (pdfjs as any).TextLayer({ textContentSource: textContent, container: tl, viewport });
    textInflight[n] = layer;
    try {
      await layer.render();
    } catch (err) {
      // A superseded build cancels its layer; only the current build reports.
      if (textInflight[n] === layer) delete textInflight[n];
      if (buildCurrent()) failPdf(errorMessage("Couldn't draw this PDF text", err));
      return;
    }
    if (textInflight[n] === layer) delete textInflight[n];
    if (!buildCurrent()) return;
    textLayerObjs[n] = layer;
    textScale[n] = atScale;
  }

  function sizeWrapper(n: number) {
    const wrap = pageEls[n];
    if (!wrap) return;
    wrap.style.width = `calc(var(--scale-factor) * ${dims[n].w}px)`;
    wrap.style.height = `calc(var(--scale-factor) * ${dims[n].h}px)`;
  }

  // The page under a client point and the point's fractional position on it,
  // read from the CURRENT layout (before a scale change applies).
  function zoomAnchorAt(origin: { x: number; y: number }) {
    let best: { page: number; rect: DOMRect } | null = null;
    let bestDistance = Infinity;
    for (const n of visible) {
      const rect = pageEls[n]?.getBoundingClientRect();
      if (!rect) continue;
      const distance = origin.y < rect.top ? rect.top - origin.y : origin.y > rect.bottom ? origin.y - rect.bottom : 0;
      if (distance < bestDistance) {
        bestDistance = distance;
        best = { page: n, rect };
      }
    }
    if (!best) {
      const rect = pageEls[curPage()]?.getBoundingClientRect();
      if (!rect) return null;
      best = { page: curPage(), rect };
    }
    const { page, rect } = best;
    if (!(rect.width > 0 && rect.height > 0)) return null;
    return { page, x: origin.x, y: origin.y, fx: (origin.x - rect.left) / rect.width, fy: (origin.y - rect.top) / rect.height };
  }

  // Every zoom step relayouts the whole document at once (one variable write)
  // and scrolls so the anchor point stays under the pointer (pdf.js
  // #setScaleUpdatePages). Old rasters stretch; in-flight renders at the old
  // scale are cancelled, and the settle re-renders double-buffered, so the page
  // never blanks.
  function onZoom() {
    // The saved/fit scale is applied before the page layout exists, and the
    // layout is then built at that scale: nothing to resize, re-anchor or
    // settle (GH #672).
    if (!pdfDoc || !pageEls[1]) {
      zoomOrigin = null;
      return;
    }
    const s = scale();
    const view = scrollRef.getBoundingClientRect();
    const origin = zoomOrigin ?? { x: view.left + view.width / 2, y: view.top };
    zoomOrigin = null;
    const anchor = zoomAnchorAt(origin);
    zoomSettling = true;
    highestPriority = null;
    pdfTiles.reset();
    scrollRef.style.setProperty("--scale-factor", String(s));
    for (const key of Object.keys(textLayerObjs)) fitTextLayer(Number(key));
    for (const key of Object.keys(inflight)) {
      const n = Number(key);
      if (inflight[n].scale !== s) tasks[n]?.cancel();
    }
    if (anchor) {
      const rect = pageEls[anchor.page]?.getBoundingClientRect();
      if (rect && rect.width > 0) {
        scrollRef.scrollLeft += rect.left + anchor.fx * rect.width - anchor.x;
        scrollRef.scrollTop += rect.top + anchor.fy * rect.height - anchor.y;
      }
    }
    lastScrollTop = scrollRef.scrollTop;
    clearTimeout(zoomTimer);
    zoomTimer = window.setTimeout(settleZoom, PDF_ZOOM_SETTLE_MS);
  }

  function settleZoom() {
    if (!pdfDoc) return;
    zoomSettling = false;
    for (const k of Object.keys(prefetchRefused)) delete prefetchRefused[Number(k)];
    updateCurPage();
    pump();
  }

  function cancelOwnedWork() {
    disposed = true;
    pdfTiles.reset();
    latestOwner(viewerRequests, "navigation");
    io?.disconnect();
    io = null;
    clearTimeout(zoomTimer);
    clearTimeout(textTimer);
    findController.cancel();
    if (scrollRaf !== undefined) {
      scrollRaf();
      scrollRaf = undefined;
    }
    for (const k of Object.keys(tasks)) tasks[Number(k)]?.cancel();
    for (const k of Object.keys(textInflight)) cancelInflightText(Number(k));
    ownerWindow.removeEventListener("mousemove", onAreaMove);
    ownerWindow.removeEventListener("mouseup", onAreaUp);
    areaDrag?.band.remove();
    areaDrag = null;
  }

  let unregisterPdfParticipant = () => {};

  onMount(async () => {
    const loadOwner = graphOwner(() => !disposed && isPdfOwnershipCurrent(owner));
    setLoadError(null);
    let restoredPage: number | null = null;
    let restoredScale: number | null = null;
    try {
      const result = await readOwned(loadOwner, backend().openPdf(props.filename, props.label, binding.backendGeneration));
      if (result.kind === "stale") return;
      const state = result.value;
      highlightState.load(state.highlights);
      restoredPage = state.page;
      restoredScale = state.scale;
    } catch (error) {
      if (!loadOwner()) return;
      highlightState.load([]);
      pushToast(msg("pdf-couldn-t-load-pdf-annotations", { detail: String(error) }), "error");
    }
    let bytes: Uint8Array;
    try {
      const result = await readOwned(loadOwner, backend().readAsset(props.filename, MAX_PDF_BYTES));
      if (result.kind === "stale") return;
      bytes = result.value;
    } catch (err) {
      if (!loadOwner()) return;
      if (errorFamily(err) === "asset-too-large")
        failPdf("This PDF is larger than 256 MiB and can't be opened safely.");
      else failPdf(errorMessage("Couldn't read this PDF asset", err));
      return;
    }
    if (!bytes.length) {
      failPdf("Couldn't read this PDF asset: file is empty");
      return;
    }
    if (bytes.byteLength > MAX_PDF_BYTES) {
      failPdf("This PDF is larger than 256 MiB and can't be opened safely.");
      return;
    }
    try {
      const loaded = await pdfjs.getDocument({ data: bytes }).promise;
      if (disposed) {
        discardPdfDocument(loaded);
        return;
      }
      pdfDoc = loaded;
    } catch (err) {
      failPdf(errorMessage("Couldn't load this PDF", err));
      return;
    }
    if (!Number.isSafeInteger(pdfDoc.numPages) || pdfDoc.numPages < 1 || pdfDoc.numPages > MAX_PDF_PAGES) {
      failPdf(`This PDF reports an unsafe page count (${pdfDoc.numPages}); at most ${MAX_PDF_PAGES} pages can be displayed.`);
      return;
    }
    // Outline parsing can be slow on large PDFs. Start it once per document,
    // but never await it on the page-one/layout path that controls first paint.
    void loadOutline(pdfDoc);
    // Measure ONLY page 1 up front (for fit-width + as the size estimate for the
    // rest). Every other page is sized from that estimate and corrected to its
    // real size the first time it renders — so first paint doesn't wait on N
    // page-dict parses. Uniform PDFs (the common case) never visibly shift.
    const doc = pdfDoc;
    let p1: pdfjs.PDFPageProxy;
    try {
      p1 = await doc.getPage(1);
      if (disposed) return;
    } catch (err) {
      failPdf(errorMessage("Couldn't read this PDF's first page", err));
      return;
    }
    const vp1 = p1.getViewport({ scale: 1 });
    const dimensionError = pageDimensionsError(1, vp1.width, vp1.height);
    if (dimensionError) {
      failPdf(dimensionError);
      return;
    }
    dims[1] = { w: vp1.width, h: vp1.height };
    dimsKnown.clear();
    dimsKnown.add(1);
    for (let n = 2; n <= doc.numPages; n++) dims[n] = { w: vp1.width, h: vp1.height };
    setScale(props.scale != null ? clampScale(props.scale) : restoredScale != null ? clampScale(restoredScale) : fitWidthScale());
    setNumPages(doc.numPages);
    buildLayout();
    const navigation = currentNavigation();
    const requestedPage = navigation.page ?? restoredPage ?? 1;
    await navigateToTarget({ ...navigation, page: requestedPage });
    if (disposed) return;
    viewStateBaseline = { page: curPage(), scale: scale() };
    viewStateReady = true;
    props.onViewState?.(viewStateBaseline);
    setReady(true);
  });

  onCleanup(() => {
    unregisterPdfParticipant();
    cancelOwnedWork();
    setOutlineOpen(false);
    setSettingsOpen(false);
    setOutlineItems([]);
    setOutlineReady(false);
    setExpandedOutlineIds(new Set<string>());
    releaseAllCanvases();
    if (pdfDoc) discardPdfDocument(pdfDoc);
    pdfDoc = null;
  });

  // Zoom changes: relayout + lazy re-raster of visible pages only.
  createEffect(on(scale, onZoom, { defer: true }));
  createEffect(on(
    () => [curPage(), scale()] as const,
    ([page, nextScale]) => scheduleViewState(page, nextScale),
    { defer: true }
  ));
  // Repaint highlight overlays whenever the set changes (rendered pages only).
  createEffect(on(highlights, () => {
    for (const n of Object.keys(renderedScale)) repaintPage(Number(n));
  }));
  // A new intent within the same asset must navigate without remounting the
  // PDF. Asset switches are handled by KeyedPdfViewer's filename key.
  // Route updates also report scrolling/zooming. Memo equality keeps those
  // updates from replaying an unchanged intent; a fresh serial still navigates
  // even when its destination is identical to the previous one.
  const navigationKey = createMemo(() => props.navigationKey?.() ?? "none");
  createEffect(
    on(
      navigationKey,
      () => {
        const target = untrack(() => props.navigation?.());
        if (viewStateReady && target?.filename === props.filename) {
          void navigateToTarget(target);
        }
      },
      { defer: true }
    )
  );

  function repaintPage(n: number) {
    const layer = hlLayers[n];
    if (!layer) return;
    layer.innerHTML = "";
    const s = scale();
    const openEdit = (id: string) => (ev: MouseEvent) => {
      ev.preventDefault();
      ev.stopPropagation();
      setMenu({ x: ev.clientX, y: ev.clientY, id }); // open the edit/remove popup
    };
    for (const h of highlights()) {
      if (h.page !== n) continue;
      // Area highlight: a single bordered rectangle over the bounding box (the
      // cropped region stays visible underneath the live page canvas), so it
      // reads as a framed area rather than a text shade.
      if (h.image != null) {
        const r = rectInPageSpace(h.position.bounding, dims[n]);
        const rgb = COLOR_RGB[h.color] ?? COLOR_RGB.yellow;
        const div = document.createElement("div");
        div.className = "pdf-hl pdf-hl-area";
        div.dataset.highlightId = h.id;
        div.classList.toggle("pdf-hl-target", h.id === activeHighlightId);
        div.style.left = `${r.left * s}px`;
        div.style.top = `${r.top * s}px`;
        div.style.width = `${r.width * s}px`;
        div.style.height = `${r.height * s}px`;
        div.style.borderColor = `rgba(${rgb}, 0.9)`;
        div.style.background = `rgba(${rgb}, 0.18)`; // translucent fill over the captured region
        div.style.cursor = "pointer";
        div.onclick = openEdit(h.id);
        div.oncontextmenu = openEdit(h.id);
        layer.appendChild(div);
        continue;
      }
      for (const storedRect of h.position.rects) {
        const r = rectInPageSpace(storedRect, dims[n]);
        const div = document.createElement("div");
        div.className = "pdf-hl";
        div.dataset.highlightId = h.id;
        div.classList.toggle("pdf-hl-target", h.id === activeHighlightId);
        div.style.left = `${r.left * s}px`;
        div.style.top = `${r.top * s}px`;
        div.style.width = `${r.width * s}px`;
        div.style.height = `${r.height * s}px`;
        div.style.background = COLOR_RGBA[h.color] ?? COLOR_RGBA.yellow;
        div.style.cursor = "pointer";
        div.onclick = openEdit(h.id);
        div.oncontextmenu = openEdit(h.id);
        layer.appendChild(div);
      }
    }
  }

  const zoomBy = (factor: number, origin?: { x: number; y: number }) => {
    const next = Math.min(4, Math.max(0.4, Math.round(scale() * factor * 100) / 100));
    if (next === scale()) return;
    zoomOrigin = origin ?? null;
    setScale(next);
  };

  // Ctrl/Cmd + wheel zooms around the pointer (like a PDF reader): wheel notches
  // and pixel deltas accumulate into ×1.1 steps, a trackpad pinch into a
  // continuous factor; modifier-added momentum tails are only consumed.
  let wheelZoomState: WheelZoomGestureState = {};
  const wheelZoom = createWheelZoomAccumulator();
  let ctrlKeyDown = false;
  const onWheel = (e: WheelEvent) => {
    pendingHighlight = null; // the reader took over the scroll position
    const decision = decideWheelZoomGesture(wheelZoomState, e.ctrlKey || e.metaKey, e.timeStamp);
    wheelZoomState = decision.state;
    if (!decision.consume) return;
    e.preventDefault();
    e.stopPropagation();
    if (!decision.zoom) return;
    const origin = { x: e.clientX, y: e.clientY };
    if (wheelZoom.isPinch(e, ctrlKeyDown)) {
      zoomBy(wheelZoom.pinchFactor(scale(), e.deltaY), origin);
      return;
    }
    const steps = wheelZoom.steps(e);
    if (steps) zoomBy(Math.pow(1.1, steps), origin);
  };
  onMount(() => {
    const onCtrl = (e: KeyboardEvent) => { if (e.key === "Control") ctrlKeyDown = e.type === "keydown"; };
    const onBlur = () => { ctrlKeyDown = false; wheelZoom.reset(); };
    ownerWindow.addEventListener("keydown", onCtrl);
    ownerWindow.addEventListener("keyup", onCtrl);
    ownerWindow.addEventListener("blur", onBlur);
    onCleanup(() => {
      ownerWindow.removeEventListener("keydown", onCtrl);
      ownerWindow.removeEventListener("keyup", onCtrl);
      ownerWindow.removeEventListener("blur", onBlur);
    });
  });

  // Ctrl/Cmd +/-/0 zoom (like every PDF reader). Active while a PDF is open;
  // preventDefault stops the webview's own page zoom.
  const onKeyZoom = (e: KeyboardEvent) => {
    if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
    if (shortcutKey(e) === "f") {
      e.preventDefault();
      openFind();
      return;
    }
    // +/-/0 zoom the PDF only when the PDF pane is focused; otherwise the notes
    // pane owns them for whole-interface zoom (see zoom.ts).
    if (!(props.focused?.() ?? activePane() === "pdf")) return;
    if (e.key === "=" || e.key === "+") {
      e.preventDefault();
      zoomBy(1.1);
    } else if (e.key === "-" || e.key === "_") {
      e.preventDefault();
      zoomBy(1 / 1.1);
    } else if (shortcutKey(e) === "0") {
      e.preventDefault();
      setScale(fitWidthScale());
    }
  };
  onMount(() => {
    ownerWindow.addEventListener("keydown", onKeyZoom);
    onCleanup(() => ownerWindow.removeEventListener("keydown", onKeyZoom));
  });

  const captureTextSelection = (target: EventTarget | null, anchor?: { x: number; y: number }) => {
    if (areaMode() || areaDrag) return;
    pendingArea = null;
    const selected = pdfTextSelection(target, anchor, dims, scale());
    if (!selected) { setMenu(null); return; }
    pending = selected;
    setMenu(selected.menu);
  };
  const onMouseUp = (e: MouseEvent) => captureTextSelection(e.target, { x: e.clientX, y: e.clientY });
  const onTouchSelectionEnd = (e: TouchEvent) => {
    pendingHighlight = null;
    if (!touchCapable()) return;
    const touch = e.changedTouches?.[0];
    queueMicrotask(() => captureTextSelection(e.target,
      touch ? { x: touch.clientX, y: touch.clientY } : undefined));
  };
  // Follows the input capability (D-20261018-W1): registered while a touch input
  // exists, removed when it goes away, so a convertible changing mode does not
  // need the PDF reopened.
  createEffect(() => {
    if (!touchCapable()) return;
    untrack(() => {
      let timer: number | undefined;
      const selectionChanged = () => {
        clearTimeout(timer);
        timer = window.setTimeout(() => {
          if (!findOpen()) captureTextSelection(viewerRootEl ?? null);
        }, 120);
      };
      ownerWindow.document.addEventListener("selectionchange", selectionChanged);
      onCleanup(() => {
        clearTimeout(timer);
        ownerWindow.document.removeEventListener("selectionchange", selectionChanged);
      });
    });
  });

  const createHighlight = async (color: string) => {
    if (!pending) return;
    if (highlightState.editBlocked()) return;
    const h: Highlight = {
      id: crypto.randomUUID(),
      page: pending.page,
      position: { page: pending.page, bounding: pending.bounding, rects: pending.rects },
      color,
      text: pending.text,
      image: null,
    };
    highlightState.edit([...highlights(), h]);
    ownerWindow.getSelection()?.removeAllRanges();
    closeHighlightMenu();
    pending = null;
    if (await persist()) await copyCreatedHighlightRef(h.id);
  };

  // --- area (image) highlights ---------------------------------------------
  // Rubber-band a rectangle over a single page; on release, crop that region of
  // the page canvas to a PNG (saved in OG's `assets/<key>/<page>_<id>_<stamp>.png`
  // layout) and create an area highlight (`text: null`, `image: <stamp>`).
  // Area capture starts when the toolbar toggle is on OR the user holds the OG
  // platform modifier: Command on macOS, Shift elsewhere.
  const areaModifier = (e: MouseEvent) => isPdfAreaModifier(e, isMac);
  const areaPoint = (wrap: HTMLElement, e: MouseEvent) => {
    const base = wrap.getBoundingClientRect();
    return {
      x: Math.max(0, Math.min(base.width, e.clientX - base.left)),
      y: Math.max(0, Math.min(base.height, e.clientY - base.top)),
    };
  };
  const onAreaDown = (e: MouseEvent) => {
    pendingHighlight = null; // includes a scrollbar drag
    if ((!areaMode() && !areaModifier(e)) || e.button !== 0) return;
    const wrap = (e.target as HTMLElement).closest(".pdf-page") as HTMLElement | null;
    if (!wrap) return;
    e.preventDefault();
    pending = null;
    closeHighlightMenu();
    const start = areaPoint(wrap, e);
    const band = document.createElement("div");
    band.className = "pdf-area-band";
    wrap.appendChild(band);
    areaDrag = { page: Number(wrap.dataset.page), wrap, startX: start.x, startY: start.y, band };
    ownerWindow.addEventListener("mousemove", onAreaMove);
    ownerWindow.addEventListener("mouseup", onAreaUp, { once: true });
  };
  const onAreaMove = (e: MouseEvent) => {
    if (!areaDrag) return;
    const { x, y } = areaPoint(areaDrag.wrap, e);
    Object.assign(areaDrag.band.style, {
      left: `${Math.min(x, areaDrag.startX)}px`,
      top: `${Math.min(y, areaDrag.startY)}px`,
      width: `${Math.abs(x - areaDrag.startX)}px`,
      height: `${Math.abs(y - areaDrag.startY)}px`,
    });
  };
  const onAreaUp = (e: MouseEvent) => {
    ownerWindow.removeEventListener("mousemove", onAreaMove);
    const drag = areaDrag;
    areaDrag = null;
    if (!drag) return;
    drag.band.remove();
    const { x, y } = areaPoint(drag.wrap, e);
    const cssWidth = Math.abs(x - drag.startX);
    const cssHeight = Math.abs(y - drag.startY);
    if (cssWidth <= 10 || cssHeight <= 10) return;
    const s = scale();
    // Rect in unscaled PDF coordinates (the same space highlight rects are stored in).
    const rect: Rect = {
      left: Math.min(x, drag.startX) / s,
      top: Math.min(y, drag.startY) / s,
      width: Math.abs(x - drag.startX) / s,
      height: Math.abs(y - drag.startY) / s,
      source_width: dims[drag.page].w,
      source_height: dims[drag.page].h,
    };
    pendingArea = { page: drag.page, wrap: drag.wrap, rect };
    setMenu({ x: e.clientX, y: e.clientY });
    setAreaMode(false);
  };

  const createAreaHighlightOwned = async (color: string): Promise<boolean> => {
    const area = pendingArea;
    if (!area) return true;
    if (highlightState.editBlocked()) return false;
    pendingArea = null;
    setMenu(null);
    const { page, wrap, rect } = area;
    const bytes = await cropPdfArea(page, wrap, rect, scale(), renderedScale[page],
      renderedPixelRatio[page] ?? 1, renderPage);
    if (!highlightGraphOwner()) return false;
    if (!bytes) {
      pushToast(msg("pdf-couldn-t-capture-that-region-try-again"), "error");
      return false;
    }
    const id = crypto.randomUUID();
    const stamp = Date.now();
    // Save the cropped PNG FIRST so the file exists before the .edn references it.
    try {
      const image = await writeOwned(highlightGraphOwner, trackAssetWrite(
        backend().savePdfAreaImage(props.filename, page, id, stamp, bytes, binding.backendGeneration)));
      if (image.kind === "stale") return false;
    } catch (e) {
      pushToast(msg("pdf-couldn-t-save-the-area-image-try-again", { detail: String(e) }), "error");
      return false;
    }
    const h: Highlight = {
      id,
      page,
      position: areaHighlightPosition(page, rect),
      color,
      text: null,
      image: stamp,
    };
    highlightState.addCrop(id, { page, stamp });
    highlightState.edit([...highlights(), h]);
    const intent = highlightState.newIntent();
    if (!(await highlightState.persistInsideMutation(intent))) return false;
    if (intent()) await copyCreatedHighlightRef(h.id);
    return true;
  };

  const createAreaHighlight = async (color: string) => {
    try {
      await trackPdfMutation(owner, () => createAreaHighlightOwned(color));
    } catch {
      // Ownership retirement cancels a not-yet-started area mutation.  It must
      // not be retried after another graph is bound.
    }
  };

  const closeSafely = async () => {
    if (await drainPdfWork()) {
      if (props.onClose) props.onClose();
      else void focusedRouter().closePdf();
    }
    else highlightState.drainBlocked();
  };

  // --- page navigation -----------------------------------------------------
  const scrollToPage = (n: number) => {
    const np = numPages() || 1;
    const p = Math.max(1, Math.min(np, Math.floor(n) || 1));
    if (pageEls[p]) scrollRef.scrollTop = pageEls[p].offsetTop;
    setCurPage(p);
    setPageField(String(p));
  };
  const activateOutlineItem = async (item: PdfOutlineItem) => {
    const doc = pdfDoc;
    if (!doc || item.destination === null) return;
    let destination: unknown = item.destination;
    if (typeof destination === "string") {
      try {
        destination = await doc.getDestination(destination);
      } catch {
        return;
      }
    }
    if (disposed || pdfDoc !== doc || !Array.isArray(destination) || !destination.length) return;
    const target = destination[0];
    if (Number.isSafeInteger(target) && Number(target) >= 0) {
      scrollToPage(Number(target) + 1);
      return;
    }
    if (!isPdfPageRef(target)) return;
    try {
      const index = await doc.getPageIndex(target);
      if (!disposed && pdfDoc === doc && Number.isSafeInteger(index) && index >= 0) scrollToPage(index + 1);
    } catch {
      // A broken outline destination is ignored without activating its URL.
    }
  };
  const commitPageField = () => {
    const v = parseInt(pageField(), 10);
    if (Number.isFinite(v)) scrollToPage(v);
  };
  // Track the page filling the viewport's upper region, and re-prioritize the
  // render queue for the new position (rAF-throttled, once per frame).
  const updateCurPage = () => {
    scrollRaf = undefined;
    pump();
    if (zoomSettling) return;
    const np = numPages();
    if (!np) return;
    const probe = scrollRef.scrollTop + scrollRef.clientHeight * 0.25;
    // Wrappers are in document order: binary-search the last one starting
    // above the probe instead of reading every page's layout.
    let n = 1, lo = 1, hi = np;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const el = pageEls[mid];
      if (!el) break;
      if (el.offsetTop <= probe) {
        n = mid;
        lo = mid + 1;
      } else hi = mid - 1;
    }
    setCurPage(n);
  };
  const onScroll = () => {
    const top = scrollRef.scrollTop;
    if (top !== lastScrollTop) {
      // A move of more than a viewport is a jump (outline, page field, link,
      // scrollbar drag), not a scroll direction: the reader reads on downward.
      scrollingDown = Math.abs(top - lastScrollTop) > scrollRef.clientHeight || top > lastScrollTop;
      lastScrollTop = top;
    }
    if (scale() > 3) for (const n of visible) {
      if (tilePages[n] && pageEls[n]) pdfTiles.refresh(tilePages[n], n, pageEls[n], scrollRef, scale());
    }
    if (scrollRaf !== undefined) return;
    scrollRaf = requestFrame(scrollRef, updateCurPage);
  };
  // Keep the page field showing the scrolled page (unless it's being edited).
  createEffect(() => {
    const c = curPage();
    if (!pageInputFocused) setPageField(String(c));
  });

  // Find scans and its bounded text cache belong to the reader view.
  const findController = createPdfFind({
    document: () => pdfDoc,
    owner: highlightGraphOwner,
    requests: viewerRequests,
    scrollToPage,
    renderPage,
    renderedScale: (page) => renderedScale[page],
    textScale: (page) => textScale[page],
    buildTextLayer,
    textLayer: (page) => textLayers[page],
    scrollElement: () => scrollRef,
    scale,
  });
  const { findOpen, findQuery, findCount, findCur, findTruncated,
    scheduleFind, nextMatch, openFind, closeFind } = findController;
  // A single-pane shell renders this viewer as the full-width PDF takeover. It
  // is a parent transient so Find, settings, outline, and highlight popovers
  // still peel first; once they are gone, either Android Back or Escape closes
  // the pane.
  createEffect(() => {
    if (!pdfTakesOverPane()) return;
    const unregister = registerTransientLayer({
      id: props.onClose ? surfaceLayerId : "pdf-pane",
      root: () => viewerRootEl ?? null,
      dismiss: () => {
        void closeSafely();
        return false;
      },
    });
    onCleanup(unregister);
  });
  createEffect(() => {
    if (!findOpen()) return;
    const unregister = registerTransientLayer({
      id: findLayerId,
      parentId: props.onClose ? surfaceLayerId : "pdf-pane",
      root: () => findRootEl ?? null,
      trigger: () => findTriggerEl ?? null,
      dismiss: () => {
        closeFind();
        return true;
      },
    });
    onCleanup(unregister);
  });
  createEffect(() => {
    if (!settingsOpen()) return;
    const unregister = registerTransientLayer({
      id: settingsLayerId,
      parentId: props.onClose ? surfaceLayerId : "pdf-pane",
      root: () => settingsRootEl ?? null,
      trigger: () => settingsTriggerEl ?? null,
      dismiss: () => {
        setSettingsOpen(false);
        return true;
      },
    });
    onCleanup(unregister);
  });
  dismissOnOutsidePointer({
    open: settingsOpen,
    inside: () => [settingsRootEl, settingsTriggerEl],
    dismiss: () => setSettingsOpen(false),
  });
  createEffect(() => {
    if (!outlineOpen()) return;
    const unregister = registerTransientLayer({
      id: outlineLayerId,
      parentId: props.onClose ? surfaceLayerId : "pdf-pane",
      root: () => outlineRootEl ?? null,
      trigger: () => outlineTriggerEl ?? null,
      dismiss: () => {
        setOutlineOpen(false);
        return true;
      },
    });
    onCleanup(unregister);
  });
  dismissOnOutsidePointer({
    open: outlineOpen,
    inside: () => [outlineRootEl, outlineTriggerEl],
    dismiss: () => setOutlineOpen(false),
  });
  createEffect(() => {
    if (!menu()) return;
    const unregister = registerTransientLayer({
      id: highlightMenuLayerId,
      parentId: props.onClose ? surfaceLayerId : "pdf-pane",
      root: () => highlightMenuRootEl ?? null,
      dismiss: () => {
        closeHighlightMenu();
        return true;
      },
    });
    onCleanup(unregister);
  });
  dismissOnOutsidePointer({
    open: () => menu() != null,
    inside: () => [highlightMenuRootEl],
    // Only a PENDING area selection is abandoned by pressing elsewhere; a menu
    // over an existing highlight stays until it is dismissed deliberately.
    dismiss: () => { if (pendingArea) closeHighlightMenu(); },
  });

  unregisterPdfParticipant = registerPdfParticipant(owner, {
    flush: async () => !highlightState.drainBlocked() && (!unsavedHighlights() || await persist()),
    cancel: cancelOwnedWork,
  });

  return <PdfViewerView {...{
    props, theme, ready, unsavedHighlights, highlightCleanupPending, curPage,
    pageField, numPages, findOpen, scale, areaMode, outlineOpen, settingsOpen,
    highlightConflict, highlightDecisionBusy, outlineReady, outlineTruncated,
    outlineItems, expandedOutlineIds, findQuery, findCount, findCur,
    findTruncated, loadError, menu, setPageField, setScale, setAreaMode,
    setOutlineOpen, setSettingsOpen, scrollToPage, commitPageField, openFind,
    closeFind, zoomBy, fitWidthScale, fitHeightScale, closeSafely,
    keepMineHighlights, useDiskHighlights, discardMineHighlights,
    retryHighlightCleanup, chooseTheme, toggleOutlineItem, activateOutlineItem,
    scheduleFind, nextMatch, onAreaDown, onMouseUp, onTouchSelectionEnd,
    onWheel, onScroll, recolorHighlight, createAreaHighlight, createHighlight,
    copyExistingHighlightRef, openExistingHighlightReferences, deleteHighlight,
    pendingArea: () => !!pendingArea,
    setPageInputFocused: (focused: boolean) => { pageInputFocused = focused; },
    setViewerRootEl: (el: HTMLDivElement) => { viewerRootEl = el; },
    setFindTriggerEl: (el: HTMLButtonElement) => { findTriggerEl = el; },
    setSettingsTriggerEl: (el: HTMLButtonElement) => { settingsTriggerEl = el; },
    setOutlineTriggerEl: (el: HTMLButtonElement) => { outlineTriggerEl = el; },
    setSettingsRootEl: (el: HTMLDivElement) => { settingsRootEl = el; },
    setOutlineRootEl: (el: HTMLDivElement) => { outlineRootEl = el; },
    setFindRootEl: (el: HTMLDivElement) => { findRootEl = el; },
    setFindInputEl: findController.setInput,
    setScrollRef: (el: HTMLDivElement) => { scrollRef = el; },
    setHighlightMenuRootEl: (el: HTMLDivElement) => { highlightMenuRootEl = el; },
  }} />;
}
