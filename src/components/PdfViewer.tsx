import { For, Show, createEffect, createSignal, createUniqueId, on, onCleanup, onMount, type JSX } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { backend } from "../backend";
import { writeClipboardText } from "../clipboard";
import { closePdf, pushToast, isConflicted, activePane, requestBlockReferences, type PdfTarget } from "../ui";
import { flushPage, isDirty, reloadHlsIfLoaded, trackAssetWrite } from "../store";
import { openPage, openPageAtBlock } from "../router";
import { areaHighlightPosition, hlsPageName, rectInPageSpace, rectWithSourceSpace, type PdfPageDimensions } from "../pdf";
import { decideWheelZoomGesture, type WheelZoomGestureState } from "../zoom";
import type { Highlight, Rect } from "../types";
import { isMac, isMobilePlatform } from "../nativeChrome";
import { registerTransientLayer } from "../transientLayers";
import {
  isPdfOwnershipCurrent,
  pdfOwnershipKey,
  registerPdfParticipant,
  trackPdfMutation,
  type PdfOwnership,
} from "../pdfOwnership";
import {
  PdfPageViewRenderer,
  TINE_PDF_LOADING_OPTIONS,
  pdfPageViewScaleToTineScale,
  type DirectPdfPageView,
} from "../pdfRenderer";
import {
  PDF_RENDERING_FINISHED,
  PDF_RENDERING_RUNNING,
  PdfRenderCoordinator,
} from "../pdfRenderCoordinator";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

const COLORS = ["yellow", "green", "blue", "red", "purple"];
const COLOR_RGB: Record<string, string> = {
  yellow: "255, 226, 86",
  green: "116, 226, 130",
  blue: "110, 176, 246",
  red: "246, 130, 130",
  purple: "190, 140, 246",
};
const COLOR_RGBA: Record<string, string> = Object.fromEntries(
  Object.entries(COLOR_RGB).map(([k, v]) => [k, `rgba(${v}, 0.4)`])
);
const PDF_THEME_KEY = "ls-pdf-viewer-theme";
const PDF_THEMES = ["light", "warm", "dark"] as const;
type PdfTheme = (typeof PDF_THEMES)[number];

interface PdfOutlineItem {
  id: string;
  label: string;
  destination: string | unknown[] | null;
  children: PdfOutlineItem[];
}

function storedPdfTheme(): PdfTheme {
  try {
    const stored = window.localStorage.getItem(PDF_THEME_KEY);
    return PDF_THEMES.includes(stored as PdfTheme) ? stored as PdfTheme : "light";
  } catch {
    return "light";
  }
}

function sanitizeOutlineItems(value: unknown, parentId = "outline"): PdfOutlineItem[] {
  if (!Array.isArray(value)) return [];
  const sanitized: PdfOutlineItem[] = [];
  value.forEach((candidate, index) => {
    if (!candidate || typeof candidate !== "object") return;
    const raw = candidate as Record<string, unknown>;
    const id = `${parentId}-${index}`;
    const label = typeof raw.title === "string" && raw.title.trim() ? raw.title : "Untitled";
    const destination = typeof raw.dest === "string" || Array.isArray(raw.dest) ? raw.dest : null;
    sanitized.push({
      id,
      label,
      destination,
      children: sanitizeOutlineItems(raw.items, id),
    });
  });
  return sanitized;
}

function isPdfPageRef(value: unknown): value is { num: number; gen: number } {
  if (!value || typeof value !== "object") return false;
  const ref = value as { num?: unknown; gen?: unknown };
  return Number.isSafeInteger(ref.num) && Number(ref.num) >= 0 &&
    Number.isSafeInteger(ref.gen) && Number(ref.gen) >= 0;
}

function PdfOutlineTree(props: {
  items: PdfOutlineItem[];
  nested?: boolean;
  expanded: (id: string) => boolean;
  toggle: (id: string) => void;
  activate: (item: PdfOutlineItem) => void;
}): JSX.Element {
  return (
    <ul class={props.nested ? "pdf-outline-children" : "pdf-outline-list"}>
      <For each={props.items}>
        {(item) => (
          <li class="pdf-outline-item">
            <div class="pdf-outline-row">
              <Show
                when={item.children.length}
                fallback={<span class="pdf-outline-disclosure-spacer" aria-hidden="true" />}
              >
                <button
                  type="button"
                  class="pdf-outline-disclosure"
                  aria-label={`${props.expanded(item.id) ? "Collapse" : "Expand"} ${item.label}`}
                  aria-expanded={props.expanded(item.id)}
                  onClick={() => props.toggle(item.id)}
                >
                  {props.expanded(item.id) ? "▾" : "▸"}
                </button>
              </Show>
              <button
                type="button"
                class="pdf-outline-label"
                disabled={item.destination === null}
                onClick={() => props.activate(item)}
              >
                {item.label}
              </button>
            </div>
            <Show when={item.children.length && props.expanded(item.id)}>
              <PdfOutlineTree
                items={item.children}
                nested
                expanded={props.expanded}
                toggle={props.toggle}
                activate={props.activate}
              />
            </Show>
          </li>
        )}
      </For>
    </ul>
  );
}

// Resource ceilings are deliberately generous for books, scanned documents, and
// architectural drawings, but bounded below the point where pdf.js/WebView canvas
// allocations can take down the whole application.
const MAX_PDF_BYTES = 256 * 1024 * 1024;
const MAX_PDF_PAGES = 5000;
const MAX_PAGE_DIMENSION = 14_400; // PDF points: 200 inches at 72 dpi.
const MAX_CANVAS_PIXELS = isMobilePlatform ? 8_388_608 : 16_777_216;
// Canvas backing stores are normally 4-byte RGBA. Bound the aggregate rather
// than counting pages: at high zoom one page can be far larger than 24 ordinary
// fit-width pages. Mobile keeps at most ~64 MiB; desktop ~192 MiB.
export const PDF_CANVAS_CACHE_PIXEL_BUDGET = isMobilePlatform ? 16_777_216 : 50_331_648;
const PDF_RENDER_COORDINATOR = new PdfRenderCoordinator(
  PDF_CANVAS_CACHE_PIXEL_BUDGET,
  MAX_CANVAS_PIXELS,
);
const PDF_FAST_SCROLL_PX_PER_MS = 2.5;
const PDF_SCROLL_SETTLE_MS = 180;
export const PDF_FIND_TEXT_CACHE_BYTES = isMobilePlatform ? 4 * 1024 * 1024 : 8 * 1024 * 1024;
export const PDF_FIND_PAGE_TEXT_BYTES = 1024 * 1024;
export const PDF_FIND_MATCH_CAP = 10_000;

export function isPdfAreaModifier(
  event: Pick<MouseEvent, "metaKey" | "shiftKey">,
  mac: boolean
): boolean {
  return mac ? event.metaKey : event.shiftKey;
}

interface Pending {
  page: number;
  rects: Rect[];
  bounding: Rect;
  text: string;
}

interface PendingArea {
  page: number;
  wrap: HTMLElement;
  rect: Rect;
}

/**
 * A PDF filename is a resource identity, not a navigation request. Key only on
 * that identity: page/highlight changes within one asset stay reactive, while
 * switching assets still tears down every document-local cache and pdf.js task.
 */
export function KeyedPdfViewer(props: { target: () => PdfTarget | null }): JSX.Element {
  const resourceKey = () => {
    const target = props.target();
    return target ? `${pdfOwnershipKey(target.owner)}:${target.filename}` : null;
  };
  return (
    <Show when={resourceKey()} keyed>
      {(_key) => {
        const target = props.target()!;
        return (
          <PdfViewer
            filename={target.filename}
            label={props.target()?.label ?? target.filename}
            owner={target.owner}
            page={props.target()?.page}
            navigation={props.target}
          />
        );
      }}
    </Show>
  );
}

export function PdfViewer(props: {
  filename: string;
  label: string;
  owner: PdfOwnership;
  page?: number;
  navigation?: () => PdfTarget | null;
}): JSX.Element {
  const owner = props.owner;
  const instanceStem = `pdf-viewer-${createUniqueId()}`;
  const findLayerId = `${instanceStem}-find`;
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
  const pageViewHosts: Record<number, HTMLDivElement> = {};
  const hlLayers: Record<number, HTMLDivElement> = {};
  const [highlights, setHighlights] = createSignal<Highlight[]>([]);
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
  // Find-in-PDF: matches are (page, char span) over each page's joined text;
  // findCur is the 1-based index of the active match (0 = none).
  const [findOpen, setFindOpen] = createSignal(false);
  const [findQuery, setFindQuery] = createSignal("");
  const [findCount, setFindCount] = createSignal(0);
  const [findCur, setFindCur] = createSignal(0);
  const [findTruncated, setFindTruncated] = createSignal(false);
  const [theme, setTheme] = createSignal<PdfTheme>(storedPdfTheme());
  const [settingsOpen, setSettingsOpen] = createSignal(false);
  const [outlineOpen, setOutlineOpen] = createSignal(false);
  const [outlineReady, setOutlineReady] = createSignal(false);
  const [outlineItems, setOutlineItems] = createSignal<PdfOutlineItem[]>([]);
  const [expandedOutlineIds, setExpandedOutlineIds] = createSignal<Set<string>>(new Set());
  let findMatches: { page: number }[] = [];
  const pageTextCache: Record<number, string> = {};
  const pageTextLru: number[] = [];
  let pageTextCacheBytes = 0;
  let findToken = 0;
  let findDebounce: number | undefined;
  let findInputEl: HTMLInputElement | undefined;
  let pending: Pending | null = null;
  let pendingArea: PendingArea | null = null;
  // The highlight ids last synced to disk (load baseline, refreshed after each
  // successful write) — sent so the backend's 3-way merge honors deletions while
  // preserving externally-added highlights.
  let baseIds: string[] = [];
  let pdfDoc: pdfjs.PDFDocumentProxy | null = null;
  let pageRenderer: PdfPageViewRenderer | null = null;
  const pageMounts: Partial<Record<number, Promise<DirectPdfPageView | null>>> = {};
  let disposed = false;
  let navigationToken = 0;
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
    setExpandedOutlineIds(new Set<string>());
    let loaded: unknown = [];
    try {
      loaded = await doc.getOutline();
    } catch {
      loaded = [];
    }
    if (disposed || pdfDoc !== doc) return;
    setOutlineItems(sanitizeOutlineItems(loaded));
    setOutlineReady(true);
  }

  // Per-page unscaled dimensions (index 1..N), fetched once so we can size every
  // page wrapper up front — that gives correct scroll geometry without having to
  // rasterize the whole document.
  const dims: { w: number; h: number }[] = [];
  let maxQueuedRenders = 0;
  let cancelledRenders = 0;
  // Pages whose REAL unscaled size has been measured (others use a page-1
  // estimate until first render), so opening a long PDF doesn't parse every page
  // dict before first paint.
  const dimsKnown = new Set<number>();
  // Pages currently intersecting the viewport — the only ones we rasterize.
  const visible = new Set<number>();
  let io: IntersectionObserver | null = null;
  let fastScrolling = false;
  let lastScrollAt = 0;
  let lastScrollTop = 0;
  let scrollingDown = true;
  let scrollSettleTimer: number | undefined;
  let zoomTimer: number | undefined;
  // Scroll anchor captured at the START of a zoom burst (pre-resize), restored
  // once on settle — so a 5×Ctrl+ burst keeps the document position without an
  // anchor calc per press.
  let zoomAnchorRatio: number | null = null;

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

  // Persist the current highlight set to disk. Returns false (and toasts) without
  // mutating the on-disk baseline if anything failed, so the caller can revert the
  // optimistic UI change rather than show a highlight that didn't actually save.
  const persistOwned = async (): Promise<boolean> => {
    const hlsName = hlsPageName(props.filename);
    // If the notes (hls__) page is open with unsaved edits, get them onto disk
    // FIRST so the backend merges against them. Otherwise this write reads a disk
    // copy that lacks them, and the reload below would drop them. Abort (don't
    // clobber) if the notes page can't be flushed.
    if (isDirty(hlsName) || isConflicted(hlsName)) {
      if (!(await flushPage(hlsName))) {
        pushToast("Couldn't save notes — highlight not written. Resolve the conflict and retry.", "error");
        return false;
      }
    }
    try {
      // Current Logseq sidecars store x1/y1/x2/y2 plus the coordinate-space page
      // dimensions. Enrich old Tine rectangles lazily on the first real edit so
      // merely opening a graph never rewrites it.
      const persisted = await highlightsForWrite(highlights());
      const ids = persisted.map((h) => h.id);
      await trackAssetWrite(
        backend().writeHighlights(props.filename, props.label, persisted, baseIds)
      );
      setHighlights(persisted);
      baseIds = ids; // what's now on disk becomes the next write's baseline
    } catch (e) {
      pushToast(`Couldn't save highlight — try again. (${String(e)})`, "error");
      return false;
    }
    // Refresh the loaded notes page (content + save baseline) to include the change.
    await reloadHlsIfLoaded(hlsName);
    return true;
  };

  const persist = async (): Promise<boolean> => {
    try {
      return await trackPdfMutation(owner, persistOwned);
    } catch {
      // A retired owner is an expected cancellation path.  The graph switch
      // already drained before retirement; never retry against a later binding.
      return false;
    }
  };

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
    openPageAtBlock(hlsPageName(props.filename), "page", id);
  };
  // Remove a highlight (and its annotation block on the hls page).
  const deleteHighlight = async (id: string) => {
    const prev = highlights();
    setHighlights(highlights().filter((h) => h.id !== id));
    closeHighlightMenu();
    if (!(await persist())) setHighlights(prev); // restore — it's still on disk
  };
  const recolorHighlight = async (id: string, color: string) => {
    const prev = highlights();
    setHighlights(highlights().map((h) => (h.id === id ? { ...h, color } : h)));
    closeHighlightMenu();
    if (!(await persist())) setHighlights(prev); // restore the previous color
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
      await trackPdfMutation(owner, () =>
        trackAssetWrite(backend().writePdfViewState(props.filename, next.page, next.scale))
      );
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
    clearTimeout(scrollSettleTimer);
    clearTimeout(findDebounce);
    pageRenderer?.dispose();
    pageRenderer = null;
    for (const page of Object.keys(pageMounts)) delete pageMounts[Number(page)];
    scrollRef?.replaceChildren();
    const doc = pdfDoc;
    pdfDoc = null;
    if (doc) void doc.destroy().catch(() => {});
    setLoadError(message);
  }

  function errorMessage(action: string, err?: unknown): string {
    const detail = err instanceof Error ? err.message : err ? String(err) : "";
    return detail ? `${action}: ${detail}` : action;
  }

  function pageDimensionsError(page: number, width: number, height: number): string | null {
    if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
      return `PDF page ${page} reports invalid dimensions (${width} x ${height}).`;
    }
    if (width > MAX_PAGE_DIMENSION || height > MAX_PAGE_DIMENSION) {
      return `PDF page ${page} is too large to render safely (${Math.round(width)} x ${Math.round(height)} points).`;
    }
    return null;
  }

  // Build all page wrappers once, sized for the current scale. Cheap: no
  // rasterization — just sized placeholders that the IntersectionObserver fills
  // in as they scroll into view.
  function buildLayout() {
    if (!pdfDoc) return;
    pageRenderer?.dispose();
    pageRenderer = new PdfPageViewRenderer({
      document: pdfDoc,
      coordinator: PDF_RENDER_COORDINATOR,
      priority: () => activePane() === "pdf" ? 0 : 10,
      onPageRendered: (pageNumber) => {
        repaintPage(pageNumber);
        updateRenderDiagnostics();
      },
      onRenderError: (_pageNumber, error) => {
        if (!disposed) failPdf(errorMessage("Couldn't render this PDF page", error));
      },
    });
    scrollRef.innerHTML = "";
    for (const k of Object.keys(pageEls)) delete pageEls[Number(k)];
    for (const k of Object.keys(pageViewHosts)) delete pageViewHosts[Number(k)];
    for (const k of Object.keys(hlLayers)) delete hlLayers[Number(k)];
    for (const k of Object.keys(pageMounts)) delete pageMounts[Number(k)];
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

      const pageViewHost = document.createElement("div");
      pageViewHost.className = "pdf-page-view-host";
      const hl = document.createElement("div");
      hl.className = "pdf-hl-layer";
      wrap.appendChild(pageViewHost);
      wrap.appendChild(hl);

      scrollRef.appendChild(wrap);
      pageEls[n] = wrap;
      pageViewHosts[n] = pageViewHost;
      hlLayers[n] = hl;
      io.observe(wrap);
    }
  }

  async function ensurePageView(n: number): Promise<DirectPdfPageView | null> {
    const existing = pageRenderer?.getPageView(n);
    if (existing) return existing;
    if (pageMounts[n]) return pageMounts[n];
    const renderer = pageRenderer;
    const host = pageViewHosts[n];
    if (!renderer || !host) return null;
    const mount = renderer.mountPage(n, host, scale()).then((view) => {
      if (!view || disposed || pageRenderer !== renderer || pageViewHosts[n] !== host) return null;
      const displayScale = pdfPageViewScaleToTineScale(view.scale);
      const width = view.width / displayScale;
      const height = view.height / displayScale;
      const dimensionError = pageDimensionsError(n, width, height);
      if (dimensionError) {
        failPdf(dimensionError);
        return null;
      }
      dims[n] = { w: width, h: height };
      dimsKnown.add(n);
      sizeWrapper(n, scale());
      repaintPage(n);
      syncVisibleRendering();
      return view;
    }).catch((error: unknown) => {
      if (!disposed) failPdf(errorMessage("Couldn't read this PDF page", error));
      return null;
    }).finally(() => {
      if (pageMounts[n] === mount) delete pageMounts[n];
    });
    pageMounts[n] = mount;
    return mount;
  }

  function onIntersect(entries: IntersectionObserverEntry[]) {
    for (const e of entries) {
      const n = Number((e.target as HTMLElement).dataset.page);
      if (e.isIntersecting) {
        visible.add(n);
      } else {
        visible.delete(n);
      }
    }
    syncVisibleRendering();
  }

  function pageIsInViewport(n: number) {
    const page = pageEls[n];
    if (!page || !scrollRef) return false;
    const rect = page.getBoundingClientRect();
    const viewport = scrollRef.getBoundingClientRect();
    return rect.bottom > viewport.top && rect.top < viewport.bottom;
  }

  function pageDistanceFromViewport(n: number) {
    const page = pageEls[n];
    if (!page || !scrollRef) return Number.POSITIVE_INFINITY;
    const rect = page.getBoundingClientRect();
    const viewport = scrollRef.getBoundingClientRect();
    return Math.abs((rect.top + rect.bottom) / 2 - (viewport.top + viewport.bottom) / 2);
  }

  function updateRenderDiagnostics() {
    if (!scrollRef) return;
    const views = [...(pageRenderer?.getCachedPageViews() ?? [])];
    const active = views.filter((view) => view.renderingState === PDF_RENDERING_RUNNING).length;
    const queued = views.filter((view) =>
      visible.has(view.id)
      && view.renderingState !== PDF_RENDERING_RUNNING
      && view.renderingState !== PDF_RENDERING_FINISHED
    ).length;
    maxQueuedRenders = Math.max(maxQueuedRenders, queued);
    scrollRef.dataset.activeRenders = String(active);
    scrollRef.dataset.queuedRenders = String(queued);
    scrollRef.dataset.maxQueuedRenders = String(maxQueuedRenders);
    scrollRef.dataset.cancelledRenders = String(cancelledRenders);
    scrollRef.dataset.fastScrolling = fastScrolling ? "true" : "false";
  }

  function syncVisibleRendering() {
    const renderer = pageRenderer;
    if (!renderer) return;
    const candidates = [...visible]
      .filter((pageNumber) => !fastScrolling || pageIsInViewport(pageNumber))
      .sort((left, right) =>
        Number(pageIsInViewport(right)) - Number(pageIsInViewport(left))
        || pageDistanceFromViewport(left) - pageDistanceFromViewport(right)
      );
    renderer.setVisiblePages(candidates, scrollingDown);
    for (const pageNumber of candidates) void ensurePageView(pageNumber);
    updateRenderDiagnostics();
  }

  async function renderPage(n: number) {
    const renderer = pageRenderer;
    const view = await ensurePageView(n);
    if (!renderer || renderer !== pageRenderer || !view) return;
    if (Math.abs(pdfPageViewScaleToTineScale(view.scale) - scale()) > 0.001) {
      renderer.updateScale(n, scale());
    }
    try {
      await renderer.renderPage(n, visible);
    } catch (error) {
      if (!disposed) failPdf(errorMessage("Couldn't render this PDF page", error));
    }
  }

  function currentNavigation(): PdfTarget {
    const target = props.navigation?.();
    return target?.filename === props.filename
      ? target
      : { filename: props.filename, label: props.label, owner, page: props.page };
  }

  async function navigateToTarget(target: PdfTarget) {
    const token = ++navigationToken;
    const highlight = target.highlightId
      ? highlights().find((candidate) => candidate.id === target.highlightId)
      : undefined;
    activeHighlightId = highlight?.id;
    repaintPage(highlight?.page ?? target.page ?? 1);
    const requestedPage = highlight?.page ?? target.page ?? 1;
    const page = pageEls[requestedPage] ? requestedPage : 1;
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
    if (disposed || token !== navigationToken) return;
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

  // Instant zoom feedback: scale the already-rendered canvas via CSS transform
  // (GPU, no raster) until the debounced re-raster at the new scale lands.
  function applyZoomTransform() {
    const s = scale();
    for (const n of visible) {
      const view = pageRenderer?.getPageView(n);
      const prev = view ? pdfPageViewScaleToTineScale(view.scale) : 0;
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
  function onZoom() {
    if (!pdfDoc) return;
    const s = scale();
    if (zoomAnchorRatio === null) {
      zoomAnchorRatio = scrollRef.scrollTop / (scrollRef.scrollHeight || 1);
    }
    for (const n of visible) sizeWrapper(n, s);
    applyZoomTransform();
    clearTimeout(zoomTimer);
    zoomTimer = window.setTimeout(settleZoom, 120);
  }

  function settleZoom() {
    if (!pdfDoc) return;
    const s = scale();
    for (let n = 1; n <= pdfDoc.numPages; n++) sizeWrapper(n, s);
    if (zoomAnchorRatio !== null) {
      scrollRef.scrollTop = zoomAnchorRatio * (scrollRef.scrollHeight || 1);
      zoomAnchorRatio = null;
    }
    for (const n of visible) {
      const view = pageRenderer?.getPageView(n);
      if (view) pageRenderer?.updateScale(n, s);
      else void ensurePageView(n);
    }
    syncVisibleRendering();
  }

  function cancelOwnedWork() {
    disposed = true;
    findToken++;
    navigationToken++;
    io?.disconnect();
    io = null;
    clearTimeout(zoomTimer);
    clearTimeout(scrollSettleTimer);
    clearTimeout(findDebounce);
    if (viewStateTimer !== undefined) {
      clearTimeout(viewStateTimer);
      viewStateTimer = undefined;
    }
    if (scrollRaf !== undefined) {
      cancelAnimationFrame(scrollRaf);
      scrollRaf = undefined;
    }
    pageRenderer?.dispose();
    pageRenderer = null;
    window.removeEventListener("mousemove", onAreaMove);
    window.removeEventListener("mouseup", onAreaUp);
    areaDrag?.band.remove();
    areaDrag = null;
  }

  let unregisterPdfParticipant = () => {};

  onMount(async () => {
    setLoadError(null);
    let restoredPage: number | null = null;
    let restoredScale: number | null = null;
    try {
      const state = await backend().openPdf(props.filename, props.label);
      if (disposed) return;
      setHighlights(state.highlights);
      restoredPage = state.page;
      restoredScale = state.scale;
    } catch (error) {
      setHighlights([]);
      pushToast(`Couldn't load PDF annotations. (${String(error)})`, "error", { sticky: true });
    }
    baseIds = highlights().map((h) => h.id); // load baseline for the 3-way merge
    let bytes: Uint8Array;
    try {
      bytes = await backend().readAsset(props.filename, MAX_PDF_BYTES);
      if (disposed) return;
    } catch (err) {
      if (String(err).includes("asset exceeds"))
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
      const loaded = await pdfjs.getDocument({ data: bytes, ...TINE_PDF_LOADING_OPTIONS }).promise;
      if (disposed) {
        void loaded.destroy().catch(() => {});
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
    setScale(restoredScale != null ? clampScale(restoredScale) : fitWidthScale());
    setNumPages(doc.numPages);
    buildLayout();
    const navigation = currentNavigation();
    const requestedPage = navigation.page ?? restoredPage ?? 1;
    await navigateToTarget({ ...navigation, page: requestedPage });
    if (disposed) return;
    viewStateBaseline = { page: curPage(), scale: scale() };
    viewStateReady = true;
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
    pageRenderer?.dispose();
    pageRenderer = null;
    const doc = pdfDoc;
    pdfDoc = null;
    if (doc) void doc.destroy().catch(() => {});
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
    for (const n of Object.keys(hlLayers)) repaintPage(Number(n));
  }));
  // A new intent within the same asset must navigate without remounting the
  // PDF. Asset switches are handled by KeyedPdfViewer's filename key.
  createEffect(
    on(
      () => props.navigation?.(),
      (target) => {
        if (pdfDoc && target?.filename === props.filename && pageEls[1]) {
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
    if (activePane() !== "pdf") return;
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

  const captureTextSelection = (
    target: EventTarget | null,
    anchor?: { x: number; y: number },
  ) => {
    // An area drag (toggle or platform modifier) owns the mouse; don't also make a text
    // highlight. `areaDrag` is still set here — onMouseUp (on .pdf-scroll) runs
    // before the window-level onAreaUp that clears it.
    if (areaMode() || areaDrag) return;
    pendingArea = null;
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !sel.toString().trim()) {
      setMenu(null);
      return;
    }
    const range = sel.getRangeAt(0);
    const clientRects = Array.from(range.getClientRects()).filter((r) => r.width > 0 && r.height > 0);
    if (!clientRects.length) return;

    const first = clientRects[0];
    const common = range.commonAncestorContainer;
    const commonElement = common instanceof Element ? common : common?.parentElement;
    const targetElement = target instanceof Element ? target : null;
    const wrap = commonElement?.closest(".pdf-page") as HTMLElement | null
      ?? targetElement?.closest(".pdf-page") as HTMLElement | null;
    const pageWrap =
      wrap ?? document.elementFromPoint(first.left, first.top)?.closest(".pdf-page") as HTMLElement | null;
    if (!pageWrap) return;
    const pageNum = Number(pageWrap.dataset.page);
    if (!Number.isSafeInteger(pageNum) || !dims[pageNum]) return;
    const base = pageWrap.getBoundingClientRect();
    const s = scale();

    const rects: Rect[] = clientRects.map((r) => ({
      left: (r.left - base.left) / s,
      top: (r.top - base.top) / s,
      width: r.width / s,
      height: r.height / s,
      source_width: dims[pageNum].w,
      source_height: dims[pageNum].h,
    }));
    const left = Math.min(...rects.map((r) => r.left));
    const top = Math.min(...rects.map((r) => r.top));
    const right = Math.max(...rects.map((r) => r.left + r.width));
    const bottom = Math.max(...rects.map((r) => r.top + r.height));
    pending = {
      page: pageNum,
      rects,
      bounding: {
        left,
        top,
        width: right - left,
        height: bottom - top,
        source_width: dims[pageNum].w,
        source_height: dims[pageNum].h,
      },
      text: sel.toString(),
    };
    const last = clientRects[clientRects.length - 1];
    setMenu({ x: anchor?.x ?? last.right, y: anchor?.y ?? last.bottom });
  };

  const onMouseUp = (e: MouseEvent) => {
    captureTextSelection(e.target, { x: e.clientX, y: e.clientY });
  };

  // Native mobile text handles do not reliably synthesize a mouseup. Touchend
  // owns the direct completion path; selectionchange catches later handle
  // adjustments after the platform selection UI settles.
  const onTouchSelectionEnd = (e: TouchEvent) => {
    if (!isMobilePlatform) return;
    const touch = e.changedTouches?.[0];
    queueMicrotask(() => captureTextSelection(
      e.target,
      touch ? { x: touch.clientX, y: touch.clientY } : undefined,
    ));
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
    const h: Highlight = {
      id: crypto.randomUUID(),
      page: pending.page,
      position: { page: pending.page, bounding: pending.bounding, rects: pending.rects },
      color,
      text: pending.text,
      image: null,
    };
    const prev = highlights();
    setHighlights([...highlights(), h]);
    window.getSelection()?.removeAllRanges();
    closeHighlightMenu();
    pending = null;
    if (!(await persist())) setHighlights(prev); // revert the optimistic add on failure
    else await copyCreatedHighlightRef(h.id);
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

  // Crop the page canvas to `rect` (unscaled coords) → PNG bytes.
  async function cropArea(page: number, wrap: HTMLElement, rect: Rect): Promise<Uint8Array | null> {
    const s = scale();
    let canvas = wrap.querySelector("canvas") as HTMLCanvasElement | null;
    // The page may have been LRU-evicted (or never rendered at this scale) — render
    // it so we crop a crisp, current bitmap.
    const view = pageRenderer?.getPageView(page);
    if (!canvas || !view || Math.abs(pdfPageViewScaleToTineScale(view.scale) - s) > 0.001) {
      await renderPage(page);
      canvas = wrap.querySelector("canvas") as HTMLCanvasElement | null;
    }
    if (!canvas) return null;
    // The canvas backing store is `unscaledWidth * s * dpr` px wide (see renderPage),
    // so one unscaled unit = `s * dpr` backing pixels.
    const dpr = canvas.width / Math.max(1, dims[page].w * s);
    const f = s * dpr;
    const sx = Math.max(0, Math.round(rect.left * f));
    const sy = Math.max(0, Math.round(rect.top * f));
    const sw = Math.min(canvas.width - sx, Math.round(rect.width * f));
    const sh = Math.min(canvas.height - sy, Math.round(rect.height * f));
    if (sw <= 0 || sh <= 0) return null;
    const crop = document.createElement("canvas");
    crop.width = sw;
    crop.height = sh;
    crop.getContext("2d")!.drawImage(canvas, sx, sy, sw, sh, 0, 0, sw, sh);
    const blob: Blob | null = await new Promise((res) => crop.toBlob(res, "image/png"));
    return blob ? new Uint8Array(await blob.arrayBuffer()) : null;
  }

  const createAreaHighlightOwned = async (color: string): Promise<boolean> => {
    const area = pendingArea;
    if (!area) return true;
    pendingArea = null;
    setMenu(null);
    const { page, wrap, rect } = area;
    const bytes = await cropArea(page, wrap, rect);
    if (!bytes) {
      pushToast("Couldn't capture that region — try again.", "error");
      return false;
    }
    const id = crypto.randomUUID();
    const stamp = Date.now();
    // Save the cropped PNG FIRST so the file exists before the .edn references it.
    try {
      await trackAssetWrite(
        backend().savePdfAreaImage(props.filename, page, id, stamp, bytes)
      );
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
    const prev = highlights();
    setHighlights([...prev, h]);
    if (!(await persistOwned())) {
      setHighlights(prev); // revert the optimistic add on failure
      return false;
    }
    await copyCreatedHighlightRef(h.id);
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

  // --- page navigation -----------------------------------------------------
  const scrollToPage = (n: number) => {
    const np = numPages() || 1;
    const p = Math.max(1, Math.min(np, Math.floor(n) || 1));
    if (pageEls[p]) scrollRef.scrollTop = pageEls[p].offsetTop;
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
    const now = performance.now();
    const top = scrollRef.scrollTop;
    const elapsed = now - lastScrollAt;
    const speed = elapsed > 0 ? Math.abs(top - lastScrollTop) / elapsed : 0;
    scrollingDown = top >= lastScrollTop;
    lastScrollAt = now;
    lastScrollTop = top;
    if (speed >= PDF_FAST_SCROLL_PX_PER_MS) {
      fastScrolling = true;
    }
    syncVisibleRendering();
    clearTimeout(scrollSettleTimer);
    scrollSettleTimer = window.setTimeout(() => {
      fastScrolling = false;
      syncVisibleRendering();
      updateRenderDiagnostics();
    }, PDF_SCROLL_SETTLE_MS);
    updateRenderDiagnostics();
    if (scrollRaf !== undefined) return;
    scrollRaf = requestAnimationFrame(updateCurPage);
  };
  // Keep the page field showing the scrolled page (unless it's being edited).
  createEffect(() => {
    const c = curPage();
    if (!pageInputFocused) setPageField(String(c));
  });

  // --- find in document ----------------------------------------------------
  function touchPageText(n: number) {
    const index = pageTextLru.indexOf(n);
    if (index >= 0) pageTextLru.splice(index, 1);
    pageTextLru.push(n);
  }
  function admitPageText(n: number, text: string) {
    const bytes = text.length * 2;
    if (bytes > PDF_FIND_PAGE_TEXT_BYTES || bytes > PDF_FIND_TEXT_CACHE_BYTES) return;
    while (pageTextCacheBytes + bytes > PDF_FIND_TEXT_CACHE_BYTES && pageTextLru.length) {
      const evicted = pageTextLru.shift()!;
      pageTextCacheBytes -= pageTextCache[evicted].length * 2;
      delete pageTextCache[evicted];
    }
    pageTextCache[n] = text;
    pageTextCacheBytes += bytes;
    touchPageText(n);
  }
  async function pageText(n: number, token: number): Promise<string | null> {
    if (pageTextCache[n] !== undefined) {
      touchPageText(n);
      return pageTextCache[n];
    }
    if (!pdfDoc) return "";
    const page = await pdfDoc.getPage(n);
    if (token !== findToken || disposed) return null;
    const tc = await page.getTextContent();
    if (token !== findToken || disposed) return null;
    let s = "";
    for (const item of tc.items as any[]) {
      const part = typeof item.str === "string" ? item.str : "";
      if ((s.length + part.length) * 2 > PDF_FIND_PAGE_TEXT_BYTES) {
        setFindTruncated(true);
        break;
      }
      s += part;
    }
    admitPageText(n, s);
    return s;
  }
  const scheduleFind = (q: string) => {
    setFindQuery(q);
    clearTimeout(findDebounce);
    findDebounce = window.setTimeout(() => void runFind(q), 180);
  };
  async function runFind(query: string) {
    const token = ++findToken;
    const q = query.trim().toLowerCase();
    if (!q || !pdfDoc) {
      findMatches = [];
      setFindCount(0);
      setFindCur(0);
      setFindTruncated(false);
      window.getSelection()?.removeAllRanges();
      return;
    }
    const acc: { page: number }[] = [];
    setFindTruncated(false);
    const np = pdfDoc.numPages;
    for (let n = 1; n <= np; n++) {
      const loadedText = await pageText(n, token);
      if (loadedText === null) return;
      const text = loadedText.toLowerCase();
      if (token !== findToken) return; // a newer query superseded this run
      let i = text.indexOf(q);
      while (i >= 0) {
        acc.push({ page: n });
        if (acc.length >= PDF_FIND_MATCH_CAP) {
          setFindTruncated(true);
          break;
        }
        i = text.indexOf(q, i + q.length);
      }
      if (acc.length >= PDF_FIND_MATCH_CAP) break;
    }
    if (token !== findToken) return;
    findMatches = acc;
    setFindCount(acc.length);
    if (acc.length) void gotoMatch(0);
    else {
      setFindCur(0);
      window.getSelection()?.removeAllRanges();
    }
  }
  const nextMatch = (delta: number) => {
    if (findMatches.length) void gotoMatch(findCur() - 1 + delta);
  };
  async function gotoMatch(i: number) {
    const len = findMatches.length;
    if (!len) return;
    const idx = ((i % len) + len) % len;
    setFindCur(idx + 1);
    const m = findMatches[idx];
    scrollToPage(m.page);
    await ensureTextLayer(m.page);
    selectOccurrence(m.page, occurrenceIndexOnPage(idx, m.page));
  }
  function occurrenceIndexOnPage(matchIdx: number, page: number): number {
    let k = -1;
    for (let i = 0; i <= matchIdx; i++) if (findMatches[i].page === page) k++;
    return k;
  }
  // Make sure a page is rasterized and its text layer built (so we can range over
  // it) — used when jumping to a match on a not-yet-rendered page.
  async function ensureTextLayer(n: number) {
    if (!pdfDoc) return;
    await renderPage(n);
  }
  // Select the occ-th occurrence of the query within page n's text layer (the
  // pdf.js text layer is transparent text over the canvas, so a DOM selection IS
  // the visible find highlight — no overlay needed). Offsets are computed against
  // the same item concatenation runFind counts, so the index lines up.
  function selectOccurrence(n: number, occ: number) {
    const tl = pageEls[n]?.querySelector<HTMLElement>(".textLayer");
    if (!tl || occ < 0) return;
    const q = findQuery().trim().toLowerCase();
    if (!q) return;
    const walker = document.createTreeWalker(tl, NodeFilter.SHOW_TEXT);
    const nodes: Text[] = [];
    const starts: number[] = [];
    let combined = "";
    for (let nd = walker.nextNode(); nd; nd = walker.nextNode()) {
      starts.push(combined.length);
      nodes.push(nd as Text);
      combined += nd.textContent ?? "";
    }
    const hay = combined.toLowerCase();
    let from = hay.indexOf(q);
    let k = 0;
    while (from >= 0 && k < occ) {
      from = hay.indexOf(q, from + q.length);
      k++;
    }
    if (from < 0) return;
    const to = from + q.length;
    const nodeAt = (offset: number) => {
      for (let i = 0; i < nodes.length; i++) {
        const len = nodes[i].textContent?.length ?? 0;
        if (offset < starts[i] + len) return { node: nodes[i], start: starts[i] };
      }
      return null;
    };
    const a = nodeAt(from);
    const b = nodeAt(to - 1);
    if (!a || !b) return;
    const range = document.createRange();
    try {
      range.setStart(a.node, from - a.start);
      range.setEnd(b.node, to - 1 - b.start + 1);
    } catch {
      return;
    }
    const sel = window.getSelection();
    sel?.removeAllRanges();
    sel?.addRange(range);
    if (typeof range.getBoundingClientRect !== "function") return;
    const rect = range.getBoundingClientRect();
    const cont = scrollRef.getBoundingClientRect();
    if (rect.top < cont.top + 48 || rect.bottom > cont.bottom - 24) {
      scrollRef.scrollTop += rect.top - cont.top - scrollRef.clientHeight * 0.3;
    }
  }
  const openFind = () => {
    setFindOpen(true);
    queueMicrotask(() => {
      findInputEl?.focus();
      findInputEl?.select();
    });
    if (findQuery().trim()) void runFind(findQuery());
  };
  const closeFind = () => {
    setFindOpen(false);
    window.getSelection()?.removeAllRanges();
  };
  // Mobile renders this viewer as the full-width PDF takeover. It is a parent
  // transient so Find, settings, outline, and highlight popovers still peel
  // first; once they are gone, either Android Back or Escape closes the pane.
  createEffect(() => {
    if (!isMobilePlatform) return;
    const unregister = registerTransientLayer({
      id: "pdf-pane",
      root: () => viewerRootEl ?? null,
      dismiss: () => {
        closePdf();
        return true;
      },
    });
    onCleanup(unregister);
  });
  createEffect(() => {
    if (!findOpen()) return;
    const unregister = registerTransientLayer({
      id: findLayerId,
      parentId: "pdf-pane",
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
      parentId: "pdf-pane",
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
      parentId: "pdf-pane",
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
      parentId: "pdf-pane",
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
    flush: flushViewState,
    cancel: cancelOwnedWork,
  });

  return (
    <div
      ref={(el) => (viewerRootEl = el)}
      class="pdf-viewer"
      data-theme={theme()}
      data-pdf-filename={props.filename}
      data-pdf-highlight-target={props.navigation?.()?.highlightId}
      data-pdf-ready={ready() ? "true" : "false"}
    >
      <div class="pdf-toolbar">
        <span class="pdf-title">{props.label}</span>
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
                pageInputFocused = true;
                e.currentTarget.select();
              }}
              onInput={(e) => setPageField(e.currentTarget.value)}
              onBlur={() => {
                pageInputFocused = false;
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
            ref={(el) => (findTriggerEl = el)}
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
            onClick={() => openPage(hlsPageName(props.filename), "page")}
          >
            Notes
          </button>
          <button
            ref={(el) => (outlineTriggerEl = el)}
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
            ref={(el) => (settingsTriggerEl = el)}
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
          <button
            type="button"
            class="icon-btn pdf-close-btn"
            title="Close PDF"
            aria-label="Close PDF"
            onClick={closePdf}
          >
            ✕
          </button>
        </div>
      </div>
      <Show when={settingsOpen()}>
        <div ref={(el) => (settingsRootEl = el)} class="pdf-settings-menu" role="dialog" aria-label="PDF settings">
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
            <button type="button" onClick={() => { openPage(hlsPageName(props.filename), "page"); setSettingsOpen(false); }}>Notes</button>
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
        <div ref={(el) => (outlineRootEl = el)} class="pdf-outline-panel" role="dialog" aria-label="Document outline">
          <div class="pdf-outline-heading">Outline</div>
          <Show when={outlineReady()} fallback={<div class="pdf-outline-loading">Loading outline…</div>}>
            <Show when={outlineItems().length} fallback={<div class="pdf-outline-empty">No outlines</div>}>
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
        <div ref={(el) => (findRootEl = el)} class="pdf-find-bar">
          <input
            ref={(el) => (findInputEl = el)}
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
          ref={scrollRef}
          onMouseDown={onAreaDown}
          onMouseUp={onMouseUp}
          onTouchEnd={onTouchSelectionEnd}
          onWheel={onWheel}
          onScroll={onScroll}
        />
      </Show>
      <Show when={menu()}>
        <div
          ref={(el) => (highlightMenuRootEl = el)}
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
                  else if (pendingArea) void createAreaHighlight(c); // create area after explicit color choice
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
