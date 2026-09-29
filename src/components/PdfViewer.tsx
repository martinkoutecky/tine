import { createEffect, createSignal, createUniqueId, on, onCleanup, onMount, untrack, type JSX } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import { sanitizeOutlineItems, type PdfOutlineItem } from "./pdfOutline";
import { PdfViewerView } from "./pdfViewerView";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { backend } from "../backend";
import { captureBinding } from "../binding";
import { graphOwner, latestOwner, readOwned, serializeDurable, writeOwned } from "../owned";
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
import { isMac, isMobilePlatform } from "../nativeChrome";
import { registerTransientLayer } from "../transientLayers";
import { createPdfHighlightState } from "./pdfHighlightState";
import { pdfTextSelection, type PdfTextSelection } from "./pdfSelection";
import { createPdfFind } from "./pdfFind";
import { createPdfTiles, PDF_TILE_PIXEL_BUDGET } from "./pdfTiles";
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
pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

export { PDF_CANVAS_CACHE_PIXEL_BUDGET, isPdfAreaModifier } from "./pdfViewerPrimitives";
export { KeyedPdfViewer } from "./KeyedPdfViewer";

/** Render the active PDF and its highlights through the owned document paths.
 * Opening may create annotation files and refresh graph state in O(graph pages),
 * plus asset listing and up to 256 MiB of PDF data. Annotation failures toast;
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
  let scrollRaf: number | undefined;
  let viewStateTimer: number | undefined;
  let viewStateReady = false;
  let viewStateBaseline: { page: number; scale: number } | null = null;
  let pendingViewState: { page: number; scale: number } | null = null;
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
  // Pages currently intersecting the viewport — the only ones we rasterize.
  const visible = new Set<number>();
  let io: IntersectionObserver | null = null;
  let zoomTimer: number | undefined;
  // Scroll anchor captured at the START of a zoom burst (pre-resize), restored
  // once on settle — so a 5×Ctrl+ burst keeps the document position without an
  // anchor calc per press.
  let zoomAnchorPage: number | null = null;
  let zoomAnchorOffsetInPage = 0;
  let zoomSettling = false;
  // The text layer (hundreds of glyph spans on a math page) is rebuilt OFF the
  // zoom hot path: the canvas sharpens immediately, the text catches up shortly
  // after the view settles. `textScale[n]` is the scale its text was built at;
  // `pendingText` holds pages whose text needs a (re)build.
  const textScale: Record<number, number> = {};
  // The live pdf.js TextLayer instance per page, so a zoom can reposition it
  // cheaply via .update({viewport}) instead of re-extracting text and recreating
  // every glyph span (the expensive work that made zoom-in janky).
  const textLayerObjs: Record<number, any> = {};
  const pendingText = new Set<number>();
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
    sizeWrapper(pageNumber, scale());
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

  const viewStateQueue = {};
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
  const highlightGraphOwner = graphOwner(() => isPdfOwnershipCurrent(owner));
  const persist = highlightState.persist;
  const useDiskHighlights = highlightState.useDiskVersion;
  const keepMineHighlights = highlightState.keepMine;
  const discardMineHighlights = highlightState.discardMine;
  const retryHighlightCleanup = highlightState.retryCleanup;

  const copyCreatedHighlightRef = async (id: string) => {
    await writeClipboardText(`((${id}))`);
    pushToast("Copied highlight ref", "success");
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

  const flushViewState = async (): Promise<boolean> => {
    if (viewStateTimer !== undefined) {
      clearTimeout(viewStateTimer);
      viewStateTimer = undefined;
    }
    const next = pendingViewState;
    if (!next || (viewStateBaseline?.page === next.page && viewStateBaseline?.scale === next.scale)) {
      pendingViewState = null;
      return true;
    }
    try {
      const result = await trackPdfMutation(owner, () => serializeDurable(viewStateQueue, highlightGraphOwner,
        () => trackAssetWrite(backend().writePdfViewState(props.filename, next.page, next.scale, binding.backendGeneration))));
      if (result.kind === "stale") return false;
      viewStateBaseline = next;
      if (pendingViewState === next) pendingViewState = null;
      return true;
    } catch (error) {
      if (isPdfOwnershipCurrent(owner)) {
        pushToast(`Couldn't save PDF view position. (${String(error)})`, "error");
      }
      return false;
    }
  };

  const scheduleViewState = (page: number, nextScale: number) => {
    if (!isPdfOwnershipCurrent(owner)) return;
    if (!viewStateReady || !Number.isFinite(nextScale) || nextScale <= 0) return;
    if (viewStateBaseline?.page === page && viewStateBaseline?.scale === nextScale) return;
    props.onViewState?.({ page, scale: nextScale });
    pendingViewState = { page, scale: nextScale };
    if (viewStateTimer !== undefined) clearTimeout(viewStateTimer);
    viewStateTimer = window.setTimeout(() => {
      if (isPdfOwnershipCurrent(owner)) void flushViewState();
    }, 4000);
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
    for (const k of Object.keys(textLayerObjs)) delete textLayerObjs[Number(k)];
    pendingText.clear();
    clearTimeout(textTimer);
    lru.length = 0;
    visible.clear();
    io?.disconnect();
    // Modest prefetch margin: render/text only pages near the viewport, so a
    // fresh open doesn't do heavy text-layer work for a whole screenful ahead.
    io = new IntersectionObserver(onIntersect, { root: scrollRef, rootMargin: "200px 0px" });

    const s = scale();
    for (let n = 1; n <= pdfDoc.numPages; n++) {
      const wrap = document.createElement("div");
      wrap.className = "pdf-page";
      wrap.dataset.page = String(n);
      wrap.style.width = `${dims[n].w * s}px`;
      wrap.style.height = `${dims[n].h * s}px`;
      wrap.style.setProperty("--scale-factor", String(s));

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
      io.observe(wrap);
    }
  }

  function onIntersect(entries: IntersectionObserverEntry[]) {
    for (const e of entries) {
      const n = Number((e.target as HTMLElement).dataset.page);
      if (e.isIntersecting) {
        visible.add(n);
        void renderPage(n);
      } else {
        visible.delete(n);
      }
    }
  }

  // Rasterize one page at the current scale (no-op if already current). Cancels
  // any in-flight raster for the page first so rapid zooms don't pile up.
  async function renderPage(n: number) {
    if (!pdfDoc) return;
    const s = scale();
    // Already rasterized at exactly this scale → just drop any transient zoom
    // transform; the bitmap is pixel-accurate. Otherwise re-raster at the CURRENT
    // scale so text is ALWAYS crisp. renderPage runs only on the debounced zoom
    // settle and on scroll-in, not per zoom step, so this re-raster is the moment
    // the page sharpens — the CSS transform (applyZoomTransform) covers the gesture
    // itself. (Re-rastering rather than upscaling a stale bitmap is what fixes the
    // blur at high zoom; it touches only the 1–3 visible pages.)
    if (renderedScale[n] === s) {
      setCanvasTransform(n, 1);
      if (s > 3 && tilePages[n] && pageEls[n]) pdfTiles.refresh(tilePages[n], n, pageEls[n], scrollRef, s);
      return;
    }
    const wrap = pageEls[n];
    if (!wrap) return;
    const generation = (renderGeneration[n] ?? 0) + 1;
    const layout = layoutGeneration;
    renderGeneration[n] = generation;
    tasks[n]?.cancel();
    delete tasks[n];

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
        sizeWrapper(n, scale());
      }
    }

    let canvas = wrap.querySelector("canvas") as HTMLCanvasElement | null;
    if (!canvas) {
      canvas = document.createElement("canvas");
      wrap.insertBefore(canvas, wrap.firstChild);
    }
    // Render into a backing store at device-pixel resolution and CSS-size it
    // back down, so text is crisp on HiDPI displays. Cap the device-pixel factor
    // at 2 — beyond that the extra pixels aren't visible but the raster cost (and
    // zoom-in lag) grows quadratically.
    const otherVisiblePixels = [...visible]
      .filter((pageNumber) => pageNumber !== n)
      .reduce((total, pageNumber) => total + (canvasPixels[pageNumber] ?? 0), 0);
    const availablePixels = Math.max(1, PDF_CANVAS_CACHE_PIXEL_BUDGET
      - (s > 3 ? PDF_TILE_PIXEL_BUDGET : 0) - otherVisiblePixels);
    const canvasSize = safeCanvasSize(viewport.width, viewport.height, availablePixels);
    if (!canvasSize) {
      failPdf(`PDF page ${n} couldn't be sized safely for rendering.`);
      return;
    }
    const nextPixels = canvasSize.width * canvasSize.height;
    makeRoomForCanvas(n, nextPixels);
    const dpr = canvasSize.ratio;
    canvas.width = canvasSize.width;
    canvas.height = canvasSize.height;
    // Reserve immediately, before pdf.js's async render, so concurrent visible
    // page renders see the allocation and cannot all admit the full budget.
    canvasPixels[n] = nextPixels;
    canvas.style.width = `${Math.floor(viewport.width)}px`;
    canvas.style.height = `${Math.floor(viewport.height)}px`;
    canvas.style.transform = "";

    const task = page.render({
      canvasContext: canvas.getContext("2d")!,
      viewport,
      transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : undefined,
    });
    tasks[n] = task;
    try {
      await task.promise;
    } catch (err) {
      if (tasks[n] === task) delete tasks[n];
      if (renderGeneration[n] !== generation || layoutGeneration !== layout) return;
      if ((err as { name?: string } | undefined)?.name === "RenderingCancelledException") return;
      failPdf(errorMessage("Couldn't render this PDF page", err));
      return;
    }
    if (tasks[n] === task) delete tasks[n];
    if (renderGeneration[n] !== generation || layoutGeneration !== layout || scale() !== s) return;

    // Canvas is crisp now — the page is usable. Rebuild the (expensive) text
    // layer off the hot path so it doesn't make every zoom step janky.
    renderedScale[n] = s;
    renderedPixelRatio[n] = dpr;
    clearTransform(n);
    repaintPage(n);
    scheduleText(n);
    touchLru(n);
    evictCanvases();
    if (s > 3) pdfTiles.refresh(page, n, wrap, scrollRef, s);
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
    retargetZoomAnchor(page);
    setCurPage(page);
    setPageField(String(page));
    pageEls[page]?.scrollIntoView({ block: "start" });

    if (!highlight) {
      for (const element of scrollRef.querySelectorAll(".pdf-hl-target")) {
        element.classList.remove("pdf-hl-target");
      }
      return;
    }

    // OG carries the highlight entity through open-block-ref! and scrolls the
    // finder to that exact highlight. Render the destination page first so the
    // overlay exists even when it was outside the lazy viewport.
    await renderPage(page);
    if (!current()) return;
    const layer = hlLayers[page];
    const exact = layer
      ? Array.from(layer.querySelectorAll<HTMLElement>(".pdf-hl"))
          .find((element) => element.dataset.highlightId === highlight.id)
      : undefined;
    for (const element of scrollRef.querySelectorAll(".pdf-hl-target")) {
      element.classList.remove("pdf-hl-target");
    }
    exact?.classList.add("pdf-hl-target");
    exact?.scrollIntoView({ block: "center", inline: "nearest" });
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
  // Free least-recently rendered off-screen pages BEFORE allocating the next
  // backing store. This prevents a valid high-zoom document from transiently
  // building the old count-based 1.5 GiB cache.
  function makeRoomForCanvas(n: number, incomingPixels: number) {
    let total = retainedCanvasPixels(n);
    let count = Object.keys(canvasPixels).filter((page) => Number(page) !== n).length;
    const incomingCount = n >= 1 ? 1 : 0;
    while (
      total + incomingPixels > PDF_CANVAS_CACHE_PIXEL_BUDGET
        - (scale() > 3 ? PDF_TILE_PIXEL_BUDGET : 0)
      || count + incomingCount > PDF_CANVAS_CACHE_PAGE_CAP
    ) {
      // Completed pages use true LRU order. Include an off-screen in-flight
      // allocation as a fallback so rapid scrolling cannot outrun the LRU.
      const candidate = lru.find((page) => page !== n && !visible.has(page))
        ?? Object.keys(canvasPixels)
          .map(Number)
          .find((page) => page !== n && !visible.has(page));
      if (candidate === undefined) break;
      total -= canvasPixels[candidate] ?? 0;
      count -= canvasPixels[candidate] === undefined ? 0 : 1;
      freePage(candidate);
      const lruIndex = lru.indexOf(candidate);
      if (lruIndex >= 0) lru.splice(lruIndex, 1);
    }
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
    const canvas = pageEls[n]?.querySelector("canvas") as HTMLCanvasElement | null;
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
    delete textLayerObjs[n];
    delete textScale[n];
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
      const r = renderedScale[n];
      if (r !== undefined) void buildTextLayer(n, r);
      return;
    }
    pendingText.add(n);
    clearTimeout(textTimer);
    textTimer = window.setTimeout(() => void buildPendingText(), 220);
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
  async function buildTextLayer(n: number, atScale: number) {
    if (!pdfDoc || !textLayers[n]) return;
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
        await existing.update({ viewport });
        textScale[n] = atScale;
        return;
      } catch {
        // pdf.js API mismatch — fall through to a full rebuild.
      }
    }

    let textContent: Awaited<ReturnType<pdfjs.PDFPageProxy["getTextContent"]>>;
    try {
      textContent = await page.getTextContent();
    } catch (err) {
      failPdf(errorMessage("Couldn't read this PDF text", err));
      return;
    }
    if (renderedScale[n] !== atScale || !textLayers[n]) return;
    const tl = textLayers[n];
    tl.innerHTML = "";
    const layer = new (pdfjs as any).TextLayer({ textContentSource: textContent, container: tl, viewport });
    await layer.render();
    textLayerObjs[n] = layer;
    textScale[n] = atScale;
  }

  function clearTransform(n: number) {
    const c = pageEls[n]?.querySelector("canvas") as HTMLCanvasElement | null;
    if (c) c.style.transform = "";
  }
  // Display an already-rasterized page at the current scale via a GPU transform
  // of its bitmap (no re-raster). factor 1 → identity (native bitmap).
  function setCanvasTransform(n: number, factor: number) {
    const c = pageEls[n]?.querySelector("canvas") as HTMLCanvasElement | null;
    if (!c) return;
    c.style.transformOrigin = "top left";
    c.style.transform = Math.abs(factor - 1) < 0.001 ? "" : `scale(${factor})`;
  }
  // Instant zoom feedback: scale the already-rendered canvas via CSS transform
  // (GPU, no raster) until the debounced re-raster at the new scale lands.
  function applyZoomTransform() {
    const s = scale();
    for (const n of visible) {
      const prev = renderedScale[n];
      const c = pageEls[n]?.querySelector("canvas") as HTMLCanvasElement | null;
      if (c && prev) {
        c.style.transformOrigin = "top left";
        c.style.transform = `scale(${s / prev})`;
      }
    }
  }

  function sizeWrapper(n: number, s: number) {
    const wrap = pageEls[n];
    if (!wrap) return;
    wrap.style.width = `${dims[n].w * s}px`;
    wrap.style.height = `${dims[n].h * s}px`;
    wrap.style.setProperty("--scale-factor", String(s));
  }

  // Per zoom step (cheap, O(visible)): size only the visible wrappers to the new
  // scale and transform their canvases, so the view tracks the zoom instantly.
  // The expensive work — resizing EVERY wrapper (scroll geometry), restoring the
  // anchor, and re-rastering — is coalesced to one debounced `settleZoom`, so a
  // burst of Ctrl+ presses does that heavy pass once, not once per press.
  function retargetZoomAnchor(page: number) {
    if (zoomAnchorPage === null) return;
    zoomAnchorPage = page;
    zoomAnchorOffsetInPage = 0;
  }

  function onZoom() {
    if (!pdfDoc) return;
    const s = scale();
    if (!zoomSettling) {
      const anchor = curPage();
      const element = pageEls[anchor];
      zoomAnchorPage = anchor;
      zoomAnchorOffsetInPage = element && element.offsetHeight > 0
        ? (scrollRef.scrollTop - element.offsetTop) / element.offsetHeight : 0;
    }
    zoomSettling = true;
    pdfTiles.reset();
    for (const n of visible) sizeWrapper(n, s);
    applyZoomTransform();
    clearTimeout(zoomTimer);
    zoomTimer = window.setTimeout(settleZoom, 120);
  }

  function settleZoom() {
    if (!pdfDoc) return;
    const s = scale();
    for (let n = 1; n <= pdfDoc.numPages; n++) sizeWrapper(n, s);
    if (zoomAnchorPage !== null) {
      const anchor = zoomAnchorPage;
      const element = pageEls[anchor];
      if (element) scrollRef.scrollTop = element.offsetTop + zoomAnchorOffsetInPage * element.offsetHeight;
      zoomAnchorPage = null;
      zoomAnchorOffsetInPage = 0;
      setCurPage(anchor);
      setPageField(String(anchor));
    }
    zoomSettling = false;
    for (const n of visible) void renderPage(n);
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
    if (viewStateTimer !== undefined) {
      clearTimeout(viewStateTimer);
      viewStateTimer = undefined;
    }
    if (scrollRaf !== undefined) {
      cancelAnimationFrame(scrollRaf);
      scrollRaf = undefined;
    }
    for (const k of Object.keys(tasks)) tasks[Number(k)]?.cancel();
    window.removeEventListener("mousemove", onAreaMove);
    window.removeEventListener("mouseup", onAreaUp);
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
      const result = await writeOwned(loadOwner,
        backend().openPdf(props.filename, props.label, "create-page", binding.backendGeneration));
      if (result.kind === "stale") return;
      const state = result.value;
      highlightState.load(state.highlights);
      restoredPage = state.page;
      restoredScale = state.scale;
    } catch (error) {
      if (loadOwner()) highlightState.load([]);
      pushToast(`Couldn't load PDF annotations. (${String(error)})`, "error");
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
    // Ordinary viewer close remains in the same graph and must persist its last
    // location. Graph switch retired the owner first, so this branch is skipped
    // there after the explicit awaited drain.
    if (isPdfOwnershipCurrent(owner) && pendingViewState) void flushViewState();
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
  createEffect(
    on(
      () => props.navigationKey?.() ?? "none",
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

  const zoomBy = (factor: number) =>
    setScale((s) => Math.min(4, Math.max(0.4, Math.round(s * factor * 100) / 100)));

  // Ctrl/Cmd + wheel zooms (like a PDF reader); modifier-added momentum tails are only consumed.
  let wheelZoomState: WheelZoomGestureState = {};
  const onWheel = (e: WheelEvent) => {
    const decision = decideWheelZoomGesture(wheelZoomState, e.ctrlKey || e.metaKey, e.timeStamp);
    wheelZoomState = decision.state;
    if (!decision.consume) return;
    e.preventDefault();
    e.stopPropagation();
    if (!decision.zoom) return;
    zoomBy(e.deltaY < 0 ? 1.1 : 1 / 1.1);
  };

  // Ctrl/Cmd +/-/0 zoom (like every PDF reader). Active while a PDF is open;
  // preventDefault stops the webview's own page zoom.
  const onKeyZoom = (e: KeyboardEvent) => {
    if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
    if (e.key === "f" || e.key === "F") {
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
    } else if (e.key === "0") {
      e.preventDefault();
      setScale(fitWidthScale());
    }
  };
  onMount(() => {
    window.addEventListener("keydown", onKeyZoom);
    onCleanup(() => window.removeEventListener("keydown", onKeyZoom));
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
    if (!isMobilePlatform) return;
    const touch = e.changedTouches?.[0];
    queueMicrotask(() => captureTextSelection(e.target,
      touch ? { x: touch.clientX, y: touch.clientY } : undefined));
  };
  onMount(() => {
    if (!isMobilePlatform) return;
    let timer: number | undefined;
    const selectionChanged = () => {
      clearTimeout(timer);
      timer = window.setTimeout(() => {
        if (!findOpen()) captureTextSelection(viewerRootEl ?? null);
      }, 120);
    };
    document.addEventListener("selectionchange", selectionChanged);
    onCleanup(() => {
      clearTimeout(timer);
      document.removeEventListener("selectionchange", selectionChanged);
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
    window.getSelection()?.removeAllRanges();
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
    window.addEventListener("mousemove", onAreaMove);
    window.addEventListener("mouseup", onAreaUp, { once: true });
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
    window.removeEventListener("mousemove", onAreaMove);
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
      pushToast("Couldn't capture that region — try again.", "error");
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
      pushToast(`Couldn't save the area image — try again. (${String(e)})`, "error");
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
    retargetZoomAnchor(p);
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
  // Track the page filling the viewport's upper region (rAF-throttled).
  const updateCurPage = () => {
    scrollRaf = undefined;
    if (zoomSettling) return;
    const np = numPages();
    if (!np) return;
    const probe = scrollRef.scrollTop + scrollRef.clientHeight * 0.25;
    let n = 1;
    for (let i = 1; i <= np; i++) {
      const el = pageEls[i];
      if (!el) continue;
      if (el.offsetTop <= probe) n = i;
      else break;
    }
    setCurPage(n);
  };
  const onScroll = () => {
    if (scale() > 3) for (const n of visible) {
      if (tilePages[n] && pageEls[n]) pdfTiles.refresh(tilePages[n], n, pageEls[n], scrollRef, scale());
    }
    if (scrollRaf !== undefined) return;
    scrollRaf = requestAnimationFrame(updateCurPage);
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
  // Mobile renders this viewer as the full-width PDF takeover. It is a parent
  // transient so Find, settings, outline, and highlight popovers still peel
  // first; once they are gone, either Android Back or Escape closes the pane.
  createEffect(() => {
    if (!isMobilePlatform) return;
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
    // The shared registry orders Escape/Back and pointer activation; individual
    // anchored popups still own their outside-pointer dismissal.
    const dismissOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (target && !settingsRootEl?.contains(target) && !settingsTriggerEl?.contains(target)) {
        setSettingsOpen(false);
      }
    };
    document.addEventListener("pointerdown", dismissOnOutsidePointer, true);
    onCleanup(() => {
      document.removeEventListener("pointerdown", dismissOnOutsidePointer, true);
      unregister();
    });
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
    // See the settings popup above: outside-click is intentionally local while
    // the registry remains the single Escape/Back ordering authority.
    const dismissOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (target && !outlineRootEl?.contains(target) && !outlineTriggerEl?.contains(target)) {
        setOutlineOpen(false);
      }
    };
    document.addEventListener("pointerdown", dismissOnOutsidePointer, true);
    onCleanup(() => {
      document.removeEventListener("pointerdown", dismissOnOutsidePointer, true);
      unregister();
    });
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
    const dismissPendingAreaOnOutsidePointer = (event: PointerEvent) => {
      if (pendingArea && !highlightMenuRootEl?.contains(event.target as Node)) closeHighlightMenu();
    };
    document.addEventListener("pointerdown", dismissPendingAreaOnOutsidePointer, true);
    onCleanup(() => {
      document.removeEventListener("pointerdown", dismissPendingAreaOnOutsidePointer, true);
      unregister();
    });
  });

  unregisterPdfParticipant = registerPdfParticipant(owner, {
    flush: async () => !highlightState.drainBlocked() && (await flushViewState()) && (!unsavedHighlights() || await persist()),
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
