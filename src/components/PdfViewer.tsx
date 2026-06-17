import { For, Show, createEffect, createSignal, on, onCleanup, onMount, type JSX } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { backend } from "../backend";
import { closePdf, refreshNotes } from "../ui";
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
  // The inner wrapper holds the rasterized content (canvas + text + highlights)
  // at its render scale; zoom is a CSS transform on this, so it's instant.
  const innerEls: Record<number, HTMLDivElement> = {};
  const textLayers: Record<number, HTMLDivElement> = {};
  const hlLayers: Record<number, HTMLDivElement> = {};
  const [highlights, setHighlights] = createSignal<Highlight[]>([]);
  const [menu, setMenu] = createSignal<{ x: number; y: number } | null>(null);
  const [scale, setScale] = createSignal(1.4);
  let pending: Pending | null = null;
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
  // Pages currently intersecting the viewport — the only ones we rasterize.
  const visible = new Set<number>();
  let io: IntersectionObserver | null = null;
  let zoomTimer: number | undefined;

  const persist = () => backend().writeHighlights(props.filename, props.label, highlights());
  const clampScale = (s: number) => Math.min(4, Math.max(0.2, s));
  const fitWidthScale = () => (dims[1] ? clampScale((scrollRef.clientWidth - 32) / dims[1].w) : 1);
  const fitHeightScale = () => (dims[1] ? clampScale((scrollRef.clientHeight - 24) / dims[1].h) : 1);

  // Position/size a page wrapper for the current DISPLAY scale, and scale its
  // (rasterized-at-renderedScale) inner content with a CSS transform. The
  // transform is GPU-cheap, so this is what makes zoom instant — no raster.
  function layoutPage(n: number, s: number) {
    const wrap = pageEls[n];
    const inner = innerEls[n];
    if (!wrap || !inner) return;
    wrap.style.width = `${dims[n].w * s}px`;
    wrap.style.height = `${dims[n].h * s}px`;
    const r = renderedScale[n];
    inner.style.transform = r ? `scale(${s / r})` : "";
  }

  // Build all page wrappers once. Cheap: no rasterization — just sized
  // placeholders the IntersectionObserver fills in as they scroll into view.
  function buildLayout() {
    if (!pdfDoc) return;
    scrollRef.innerHTML = "";
    for (const m of [pageEls, innerEls, textLayers, hlLayers, renderedScale]) {
      for (const k of Object.keys(m)) delete (m as Record<number, unknown>)[Number(k)];
    }
    visible.clear();
    io?.disconnect();
    io = new IntersectionObserver(onIntersect, { root: scrollRef, rootMargin: "400px 0px" });

    const s = scale();
    for (let n = 1; n <= pdfDoc.numPages; n++) {
      const wrap = document.createElement("div");
      wrap.className = "pdf-page";
      wrap.dataset.page = String(n);

      const inner = document.createElement("div");
      inner.className = "pdf-page-inner";
      const textLayer = document.createElement("div");
      textLayer.className = "textLayer";
      const hl = document.createElement("div");
      hl.className = "pdf-hl-layer";
      inner.appendChild(textLayer);
      inner.appendChild(hl);
      wrap.appendChild(inner);

      scrollRef.appendChild(wrap);
      pageEls[n] = wrap;
      innerEls[n] = inner;
      textLayers[n] = textLayer;
      hlLayers[n] = hl;
      layoutPage(n, s);
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
        // Free the bitmap of pages scrolled well away (keeps a 100+-page PDF
        // from accumulating dozens of large canvases); re-renders on return.
        teardownPage(n);
      }
    }
  }

  // Drop a page's rasterized content but keep its sized wrapper.
  function teardownPage(n: number) {
    if (renderedScale[n] === undefined) return;
    tasks[n]?.cancel();
    delete tasks[n];
    delete renderedScale[n];
    innerEls[n]?.querySelector("canvas")?.remove();
    if (textLayers[n]) textLayers[n].innerHTML = "";
    if (hlLayers[n]) hlLayers[n].innerHTML = "";
  }

  // Rasterize one page at the current scale. Skips if the existing bitmap is
  // already at (or finer than) the display scale — so zooming OUT never rasters,
  // and zooming IN rasters only once the view settles. Cancels any in-flight
  // raster first so rapid zooms don't pile up.
  async function renderPage(n: number) {
    if (!pdfDoc) return;
    const s = scale();
    const r = renderedScale[n];
    if (r !== undefined && s <= r + 0.001) {
      layoutPage(n, s); // bitmap is fine (equal or being downscaled) — just place it
      return;
    }
    const inner = innerEls[n];
    if (!inner) return;
    tasks[n]?.cancel();
    delete tasks[n];

    const page = await pdfDoc.getPage(n);
    if (scale() < s - 0.001 || !innerEls[n]) return; // zoomed out again while awaiting
    const viewport = page.getViewport({ scale: s });

    let canvas = inner.querySelector("canvas") as HTMLCanvasElement | null;
    if (!canvas) {
      canvas = document.createElement("canvas");
      inner.insertBefore(canvas, inner.firstChild);
    }
    // Backing store at device-pixel resolution, CSS-sized down → crisp glyphs on
    // HiDPI without fuzzy strokes.
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.ceil(viewport.width * dpr);
    canvas.height = Math.ceil(viewport.height * dpr);
    canvas.style.width = `${Math.floor(viewport.width)}px`;
    canvas.style.height = `${Math.floor(viewport.height)}px`;

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
    delete tasks[n];

    const textContent = await page.getTextContent();
    if (!textLayers[n]) return;
    const tl = textLayers[n];
    tl.innerHTML = "";
    const layer = new (pdfjs as any).TextLayer({ textContentSource: textContent, container: tl, viewport });
    await layer.render();

    // Inner content is now 1:1 at scale s; size it and clear the zoom transform.
    inner.style.width = `${viewport.width}px`;
    inner.style.height = `${viewport.height}px`;
    renderedScale[n] = s;
    layoutPage(n, scale());
    repaintPage(n);
  }

  // Zoom = instant CSS transform on every page's inner content + a wrapper
  // resize (so scroll geometry stays right). The actual re-raster (only of
  // visible pages that are now upscaled) is debounced until the view settles.
  function onZoom() {
    if (!pdfDoc) return;
    const s = scale();
    const prevH = scrollRef.scrollHeight || 1;
    const ratio = scrollRef.scrollTop / prevH;
    for (let n = 1; n <= pdfDoc.numPages; n++) layoutPage(n, s);
    scrollRef.scrollTop = ratio * (scrollRef.scrollHeight || 1);
    clearTimeout(zoomTimer);
    zoomTimer = window.setTimeout(() => {
      for (const n of visible) void renderPage(n);
    }, 180);
  }

  onMount(async () => {
    setHighlights(await backend().readHighlights(props.filename));
    let bytes: Uint8Array;
    try {
      bytes = await backend().readAsset(props.filename);
    } catch {
      return;
    }
    if (!bytes.length) return;
    pdfDoc = await pdfjs.getDocument({ data: bytes }).promise;
    // Fetch every page's unscaled size once (getPage parses the page dict, not
    // its content — cheap relative to rendering) so wrappers size correctly.
    for (let n = 1; n <= pdfDoc.numPages; n++) {
      const vp = (await pdfDoc.getPage(n)).getViewport({ scale: 1 });
      dims[n] = { w: vp.width, h: vp.height };
    }
    setScale(fitWidthScale());
    buildLayout();
    if (props.page && pageEls[props.page]) {
      pageEls[props.page].scrollIntoView({ block: "start" });
    }
  });

  onCleanup(() => {
    io?.disconnect();
    clearTimeout(zoomTimer);
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
    // Highlights live inside the inner wrapper, which is rasterized at
    // renderedScale and CSS-scaled to the display scale — so paint them in the
    // render-scale space (the transform handles the rest).
    const s = renderedScale[n] ?? scale();
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
    setHighlights([...highlights(), h]);
    window.getSelection()?.removeAllRanges();
    setMenu(null);
    pending = null;
    await persist();
    // Let an open notes (hls__) page reload to show the new highlight.
    refreshNotes(hlsPageName(props.filename));
  };

  return (
    <div class="pdf-viewer">
      <div class="pdf-toolbar">
        <span class="pdf-title">{props.label}</span>
        <div class="pdf-toolbar-actions">
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
      <div class="pdf-scroll" ref={scrollRef} onMouseUp={onMouseUp} onWheel={onWheel} />
      <Show when={menu()}>
        <div class="pdf-color-menu" style={{ left: `${menu()!.x}px`, top: `${menu()!.y + 8}px` }}>
          <For each={COLORS}>
            {(c) => (
              <button
                class="pdf-color-swatch"
                style={{ background: COLOR_RGBA[c] }}
                onMouseDown={(e) => {
                  e.preventDefault();
                  void createHighlight(c);
                }}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
