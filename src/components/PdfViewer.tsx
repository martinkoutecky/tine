import { For, Show, createEffect, createSignal, on, onCleanup, onMount, type JSX } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { backend } from "../backend";
import { closePdf, pushToast, isConflicted } from "../ui";
import { flushPage, isDirty, reloadHlsIfLoaded } from "../store";
import { openPage } from "../router";
import { hlsPageName } from "../pdf";
import type { Highlight, Rect } from "../types";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

const COLORS = ["yellow", "green", "blue", "red", "purple"];
const COLOR_RGBA: Record<string, string> = {
  yellow: "rgba(255, 226, 86, 0.4)",
  green: "rgba(116, 226, 130, 0.4)",
  blue: "rgba(110, 176, 246, 0.4)",
  red: "rgba(246, 130, 130, 0.4)",
  purple: "rgba(190, 140, 246, 0.4)",
};

interface Pending {
  page: number;
  rects: Rect[];
  bounding: Rect;
  text: string;
}

export function PdfViewer(props: { filename: string; label: string; page?: number }): JSX.Element {
  let scrollRef!: HTMLDivElement;
  const pageEls: Record<number, HTMLDivElement> = {};
  const textLayers: Record<number, HTMLDivElement> = {};
  const hlLayers: Record<number, HTMLDivElement> = {};
  const [highlights, setHighlights] = createSignal<Highlight[]>([]);
  // The create-highlight popup (no `id`) OR the edit popup for an existing
  // highlight (`id` set → offers recolor + remove).
  const [menu, setMenu] = createSignal<{ x: number; y: number; id?: string } | null>(null);
  const [scale, setScale] = createSignal(1.4);
  // Page indicator: total pages + the page currently filling the viewport, and a
  // separately-tracked editable field (so a scroll doesn't fight the user typing).
  const [numPages, setNumPages] = createSignal(0);
  const [curPage, setCurPage] = createSignal(1);
  const [pageField, setPageField] = createSignal("1");
  let pageInputFocused = false;
  let scrollRaf: number | undefined;
  // Find-in-PDF: matches are (page, char span) over each page's joined text;
  // findCur is the 1-based index of the active match (0 = none).
  const [findOpen, setFindOpen] = createSignal(false);
  const [findQuery, setFindQuery] = createSignal("");
  const [findCount, setFindCount] = createSignal(0);
  const [findCur, setFindCur] = createSignal(0);
  let findMatches: { page: number }[] = [];
  const pageTextCache: Record<number, string> = {};
  let findToken = 0;
  let findDebounce: number | undefined;
  let findInputEl: HTMLInputElement | undefined;
  let pending: Pending | null = null;
  // The highlight ids last synced to disk (load baseline, refreshed after each
  // successful write) — sent so the backend's 3-way merge honors deletions while
  // preserving externally-added highlights.
  let baseIds: string[] = [];
  let pdfDoc: pdfjs.PDFDocumentProxy | null = null;

  // Per-page unscaled dimensions (index 1..N), fetched once so we can size every
  // page wrapper up front — that gives correct scroll geometry without having to
  // rasterize the whole document.
  const dims: { w: number; h: number }[] = [];
  // The scale a page's canvas was last rasterized at (absent = never). Used to
  // skip work and to detect a page that's stale after a zoom.
  const renderedScale: Record<number, number> = {};
  // Live render tasks (so a zoom mid-render can cancel the stale raster).
  const tasks: Record<number, pdfjs.RenderTask> = {};
  // Pages whose REAL unscaled size has been measured (others use a page-1
  // estimate until first render), so opening a long PDF doesn't parse every page
  // dict before first paint.
  const dimsKnown = new Set<number>();
  // Rendered pages in recency order (LRU). A canvas + text layer is freed once we
  // exceed CANVAS_CAP so a long PDF doesn't keep a bitmap for every page ever
  // viewed; the wrapper stays (sized) and re-renders on scroll-back. Small papers
  // never hit the cap, so they keep every canvas (instant scroll-back).
  const lru: number[] = [];
  const CANVAS_CAP = 24;
  // Pages currently intersecting the viewport — the only ones we rasterize.
  const visible = new Set<number>();
  let io: IntersectionObserver | null = null;
  let zoomTimer: number | undefined;
  // Scroll anchor captured at the START of a zoom burst (pre-resize), restored
  // once on settle — so a 5×Ctrl+ burst keeps the document position without an
  // anchor calc per press.
  let zoomAnchorRatio: number | null = null;
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

  // Persist the current highlight set to disk. Returns false (and toasts) without
  // mutating the on-disk baseline if anything failed, so the caller can revert the
  // optimistic UI change rather than show a highlight that didn't actually save.
  const persist = async (): Promise<boolean> => {
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
    const ids = highlights().map((h) => h.id);
    try {
      await backend().writeHighlights(props.filename, props.label, highlights(), baseIds);
    } catch (e) {
      pushToast(`Couldn't save highlight — try again. (${String(e)})`, "error");
      return false;
    }
    baseIds = ids; // what's now on disk becomes the next write's baseline
    // Refresh the loaded notes page (content + save baseline) to include the change.
    await reloadHlsIfLoaded(hlsName);
    return true;
  };
  // Remove a highlight (and its annotation block on the hls page).
  const deleteHighlight = async (id: string) => {
    const prev = highlights();
    setHighlights(highlights().filter((h) => h.id !== id));
    setMenu(null);
    if (!(await persist())) setHighlights(prev); // restore — it's still on disk
  };
  const recolorHighlight = async (id: string, color: string) => {
    const prev = highlights();
    setHighlights(highlights().map((h) => (h.id === id ? { ...h, color } : h)));
    setMenu(null);
    if (!(await persist())) setHighlights(prev); // restore the previous color
  };
  const clampScale = (s: number) => Math.min(4, Math.max(0.2, s));
  const fitWidthScale = () => (dims[1] ? clampScale((scrollRef.clientWidth - 32) / dims[1].w) : 1);
  const fitHeightScale = () => (dims[1] ? clampScale((scrollRef.clientHeight - 24) / dims[1].h) : 1);

  // Build all page wrappers once, sized for the current scale. Cheap: no
  // rasterization — just sized placeholders that the IntersectionObserver fills
  // in as they scroll into view.
  function buildLayout() {
    if (!pdfDoc) return;
    scrollRef.innerHTML = "";
    for (const k of Object.keys(pageEls)) delete pageEls[Number(k)];
    for (const k of Object.keys(textLayers)) delete textLayers[Number(k)];
    for (const k of Object.keys(hlLayers)) delete hlLayers[Number(k)];
    for (const k of Object.keys(renderedScale)) delete renderedScale[Number(k)];
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
      return;
    }
    const wrap = pageEls[n];
    if (!wrap) return;
    tasks[n]?.cancel();
    delete tasks[n];

    const page = await pdfDoc.getPage(n);
    if (scale() !== s || !pageEls[n]) return; // zoomed again while awaiting
    const viewport = page.getViewport({ scale: s });
    // First time we touch this page, learn its real unscaled size and correct the
    // wrapper if the page-1 estimate was off (non-uniform PDF).
    if (!dimsKnown.has(n)) {
      dimsKnown.add(n);
      const rw = viewport.width / s;
      const rh = viewport.height / s;
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
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.ceil(viewport.width * dpr);
    canvas.height = Math.ceil(viewport.height * dpr);
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
    } catch {
      return; // cancelled by a newer zoom
    }
    if (scale() !== s) return;
    delete tasks[n];

    // Canvas is crisp now — the page is usable. Rebuild the (expensive) text
    // layer off the hot path so it doesn't make every zoom step janky.
    renderedScale[n] = s;
    clearTransform(n);
    repaintPage(n);
    scheduleText(n);
    touchLru(n);
    evictCanvases();
  }

  // Record `n` as most-recently rendered.
  function touchLru(n: number) {
    const i = lru.indexOf(n);
    if (i >= 0) lru.splice(i, 1);
    lru.push(n);
  }
  // Free canvases/text for the least-recently rendered OFF-SCREEN pages once we're
  // over the cap. The wrapper (and its size) stays, so scroll geometry is intact
  // and the page re-renders when scrolled back into view.
  function evictCanvases() {
    let i = 0;
    while (lru.length > CANVAS_CAP && i < lru.length) {
      const n = lru[i];
      if (visible.has(n)) {
        i++;
        continue;
      }
      freePage(n);
      lru.splice(i, 1);
    }
  }
  function freePage(n: number) {
    tasks[n]?.cancel();
    delete tasks[n];
    pageEls[n]?.querySelector("canvas")?.remove();
    delete renderedScale[n];
    if (textLayers[n]) textLayers[n].innerHTML = "";
    delete textLayerObjs[n];
    delete textScale[n];
    pendingText.delete(n);
  }

  // Coalesced, deferred text-layer (re)build. Runs ~after the view settles, only
  // for visible pages whose text isn't already at the page's current scale.
  function scheduleText(n: number) {
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
    const page = await pdfDoc.getPage(n);
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

    const textContent = await page.getTextContent();
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
    for (const n of visible) void renderPage(n);
  }

  onMount(async () => {
    setHighlights(await backend().readHighlights(props.filename));
    baseIds = highlights().map((h) => h.id); // load baseline for the 3-way merge
    let bytes: Uint8Array;
    try {
      bytes = await backend().readAsset(props.filename);
    } catch {
      return;
    }
    if (!bytes.length) return;
    pdfDoc = await pdfjs.getDocument({ data: bytes }).promise;
    // Measure ONLY page 1 up front (for fit-width + as the size estimate for the
    // rest). Every other page is sized from that estimate and corrected to its
    // real size the first time it renders — so first paint doesn't wait on N
    // page-dict parses. Uniform PDFs (the common case) never visibly shift.
    const doc = pdfDoc;
    const p1 = await doc.getPage(1);
    const vp1 = p1.getViewport({ scale: 1 });
    dims[1] = { w: vp1.width, h: vp1.height };
    dimsKnown.clear();
    dimsKnown.add(1);
    for (let n = 2; n <= doc.numPages; n++) dims[n] = { w: vp1.width, h: vp1.height };
    setScale(fitWidthScale());
    setNumPages(doc.numPages);
    buildLayout();
    const startPage = props.page && pageEls[props.page] ? props.page : 1;
    setCurPage(startPage);
    setPageField(String(startPage));
    if (props.page && pageEls[props.page]) {
      pageEls[props.page].scrollIntoView({ block: "start" });
    }
  });

  onCleanup(() => {
    io?.disconnect();
    clearTimeout(zoomTimer);
    clearTimeout(textTimer);
    clearTimeout(findDebounce);
    if (scrollRaf !== undefined) cancelAnimationFrame(scrollRaf);
    for (const k of Object.keys(tasks)) tasks[Number(k)]?.cancel();
  });

  // Zoom changes: relayout + lazy re-raster of visible pages only.
  createEffect(on(scale, onZoom, { defer: true }));
  // Repaint highlight overlays whenever the set changes (rendered pages only).
  createEffect(on(highlights, () => {
    for (const n of Object.keys(renderedScale)) repaintPage(Number(n));
  }));
  // Jump to a highlight's page when asked while the viewer is already open.
  createEffect(
    on(
      () => props.page,
      (p) => {
        if (p && pageEls[p]) pageEls[p].scrollIntoView({ block: "start" });
      },
      { defer: true }
    )
  );

  function repaintPage(n: number) {
    const layer = hlLayers[n];
    if (!layer) return;
    layer.innerHTML = "";
    const s = scale();
    for (const h of highlights()) {
      if (h.page !== n) continue;
      for (const r of h.position.rects) {
        const div = document.createElement("div");
        div.className = "pdf-hl";
        div.style.left = `${r.left * s}px`;
        div.style.top = `${r.top * s}px`;
        div.style.width = `${r.width * s}px`;
        div.style.height = `${r.height * s}px`;
        div.style.background = COLOR_RGBA[h.color] ?? COLOR_RGBA.yellow;
        div.style.cursor = "pointer";
        div.onclick = (ev) => {
          ev.stopPropagation();
          setMenu({ x: ev.clientX, y: ev.clientY, id: h.id }); // open the edit/remove popup
        };
        layer.appendChild(div);
      }
    }
  }

  const zoomBy = (factor: number) =>
    setScale((s) => Math.min(4, Math.max(0.4, Math.round(s * factor * 100) / 100)));

  // Ctrl/Cmd + wheel zooms (like a PDF reader); plain wheel scrolls normally.
  const onWheel = (e: WheelEvent) => {
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();
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

  const onMouseUp = (e: MouseEvent) => {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !sel.toString().trim()) {
      setMenu(null);
      return;
    }
    const range = sel.getRangeAt(0);
    const clientRects = Array.from(range.getClientRects()).filter((r) => r.width > 0 && r.height > 0);
    if (!clientRects.length) return;

    const first = clientRects[0];
    const wrap = (e.target as HTMLElement).closest(".pdf-page") as HTMLElement | null;
    const pageWrap =
      wrap ?? document.elementFromPoint(first.left, first.top)?.closest(".pdf-page") as HTMLElement | null;
    if (!pageWrap) return;
    const pageNum = Number(pageWrap.dataset.page);
    const base = pageWrap.getBoundingClientRect();
    const s = scale();

    const rects: Rect[] = clientRects.map((r) => ({
      left: (r.left - base.left) / s,
      top: (r.top - base.top) / s,
      width: r.width / s,
      height: r.height / s,
    }));
    const left = Math.min(...rects.map((r) => r.left));
    const top = Math.min(...rects.map((r) => r.top));
    const right = Math.max(...rects.map((r) => r.left + r.width));
    const bottom = Math.max(...rects.map((r) => r.top + r.height));
    pending = {
      page: pageNum,
      rects,
      bounding: { left, top, width: right - left, height: bottom - top },
      text: sel.toString(),
    };
    setMenu({ x: e.clientX, y: e.clientY });
  };

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
    setMenu(null);
    pending = null;
    if (!(await persist())) setHighlights(prev); // revert the optimistic add on failure
  };

  // --- page navigation -----------------------------------------------------
  const scrollToPage = (n: number) => {
    const np = numPages() || 1;
    const p = Math.max(1, Math.min(np, Math.floor(n) || 1));
    if (pageEls[p]) scrollRef.scrollTop = pageEls[p].offsetTop;
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
    if (scrollRaf !== undefined) return;
    scrollRaf = requestAnimationFrame(updateCurPage);
  };
  // Keep the page field showing the scrolled page (unless it's being edited).
  createEffect(() => {
    const c = curPage();
    if (!pageInputFocused) setPageField(String(c));
  });

  // --- find in document ----------------------------------------------------
  async function pageText(n: number): Promise<string> {
    if (pageTextCache[n] !== undefined) return pageTextCache[n];
    if (!pdfDoc) return "";
    const page = await pdfDoc.getPage(n);
    const tc = await page.getTextContent();
    const s = (tc.items as any[]).map((it) => (typeof it.str === "string" ? it.str : "")).join("");
    pageTextCache[n] = s;
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
      window.getSelection()?.removeAllRanges();
      return;
    }
    const acc: { page: number }[] = [];
    const np = pdfDoc.numPages;
    for (let n = 1; n <= np; n++) {
      const text = (await pageText(n)).toLowerCase();
      if (token !== findToken) return; // a newer query superseded this run
      let i = text.indexOf(q);
      while (i >= 0) {
        acc.push({ page: n });
        i = text.indexOf(q, i + q.length);
      }
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
    const s = scale();
    if (renderedScale[n] !== s) await renderPage(n);
    if (renderedScale[n] !== undefined && textScale[n] !== renderedScale[n]) {
      await buildTextLayer(n, renderedScale[n]);
    }
  }
  // Select the occ-th occurrence of the query within page n's text layer (the
  // pdf.js text layer is transparent text over the canvas, so a DOM selection IS
  // the visible find highlight — no overlay needed). Offsets are computed against
  // the same item concatenation runFind counts, so the index lines up.
  function selectOccurrence(n: number, occ: number) {
    const tl = textLayers[n];
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

  return (
    <div class="pdf-viewer">
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
            <button class="icon-btn" title="Fit width" onClick={() => setScale(fitWidthScale())}>
              ↔
            </button>
            <button class="icon-btn" title="Fit height" onClick={() => setScale(fitHeightScale())}>
              ↕
            </button>
          </div>
          <button
            class="pdf-notes-btn"
            title="Open highlights & notes page"
            onClick={() => openPage(hlsPageName(props.filename), "page")}
          >
            Notes
          </button>
          <button class="icon-btn" title="Close PDF" onClick={closePdf}>
            ✕
          </button>
        </div>
      </div>
      <Show when={findOpen()}>
        <div class="pdf-find-bar">
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
              } else if (e.key === "Escape") {
                e.preventDefault();
                closeFind();
              }
            }}
          />
          <span class="pdf-find-count">
            {findCount() ? `${findCur()} / ${findCount()}` : findQuery().trim() ? "No results" : ""}
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
      <div class="pdf-scroll" ref={scrollRef} onMouseUp={onMouseUp} onWheel={onWheel} onScroll={onScroll} />
      <Show when={menu()}>
        <div class="pdf-color-menu" style={{ left: `${menu()!.x}px`, top: `${menu()!.y + 8}px` }}>
          <For each={COLORS}>
            {(c) => (
              <button
                class="pdf-color-swatch"
                style={{ background: COLOR_RGBA[c] }}
                onMouseDown={(e) => {
                  e.preventDefault();
                  const m = menu()!;
                  if (m.id) void recolorHighlight(m.id, c); // recolor existing
                  else void createHighlight(c); // create new
                }}
              />
            )}
          </For>
          <Show when={menu()!.id}>
            <button
              class="pdf-hl-remove"
              title="Remove highlight"
              onMouseDown={(e) => {
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
