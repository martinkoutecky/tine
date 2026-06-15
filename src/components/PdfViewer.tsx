import { For, Show, createEffect, createSignal, on, onMount, type JSX } from "solid-js";
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
  const [highlights, setHighlights] = createSignal<Highlight[]>([]);
  const [menu, setMenu] = createSignal<{ x: number; y: number } | null>(null);
  const [scale, setScale] = createSignal(1.4);
  let pending: Pending | null = null;
  const hlLayers: Record<number, HTMLDivElement> = {};
  let pdfDoc: pdfjs.PDFDocumentProxy | null = null;
  let rendering = false;
  let rerenderQueued = false;
  // Page-1 viewport size at scale 1, for fit-to-width / fit-to-height.
  let basePW = 0;
  let basePH = 0;

  const persist = () => backend().writeHighlights(props.filename, props.label, highlights());

  // (Re)render every page at the current scale, preserving scroll position.
  async function renderPages() {
    if (!pdfDoc) return;
    if (rendering) {
      rerenderQueued = true;
      return;
    }
    rendering = true;
    const s = scale();
    const prevH = scrollRef.scrollHeight || 1;
    const ratio = scrollRef.scrollTop / prevH;
    scrollRef.innerHTML = "";
    for (const k of Object.keys(pageEls)) delete pageEls[Number(k)];
    for (const k of Object.keys(hlLayers)) delete hlLayers[Number(k)];

    for (let n = 1; n <= pdfDoc.numPages; n++) {
      const page = await pdfDoc.getPage(n);
      const viewport = page.getViewport({ scale: s });
      const wrap = document.createElement("div");
      wrap.className = "pdf-page";
      wrap.style.width = `${viewport.width}px`;
      wrap.style.height = `${viewport.height}px`;
      wrap.style.setProperty("--scale-factor", String(s));

      const canvas = document.createElement("canvas");
      canvas.width = viewport.width;
      canvas.height = viewport.height;
      wrap.appendChild(canvas);

      const textLayer = document.createElement("div");
      textLayer.className = "textLayer";
      wrap.appendChild(textLayer);

      const hl = document.createElement("div");
      hl.className = "pdf-hl-layer";
      wrap.appendChild(hl);
      hlLayers[n] = hl;

      scrollRef.appendChild(wrap);
      wrap.dataset.page = String(n);
      pageEls[n] = wrap;

      await page.render({ canvasContext: canvas.getContext("2d")!, viewport }).promise;
      const textContent = await page.getTextContent();
      const tl = new (pdfjs as any).TextLayer({ textContentSource: textContent, container: textLayer, viewport });
      await tl.render();
    }
    repaint();
    scrollRef.scrollTop = ratio * (scrollRef.scrollHeight || 1);
    rendering = false;
    if (rerenderQueued) {
      rerenderQueued = false;
      void renderPages();
    }
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
    const first = await pdfDoc.getPage(1);
    const vp1 = first.getViewport({ scale: 1 });
    basePW = vp1.width;
    basePH = vp1.height;
    setScale(fitWidthScale()); // fit to pane width initially
    await renderPages();
    if (props.page && pageEls[props.page]) {
      pageEls[props.page].scrollIntoView({ block: "start" });
    }
  });

  const clampScale = (s: number) => Math.min(4, Math.max(0.2, s));
  const fitWidthScale = () => (basePW ? clampScale((scrollRef.clientWidth - 32) / basePW) : 1);
  const fitHeightScale = () => (basePH ? clampScale((scrollRef.clientHeight - 24) / basePH) : 1);

  // Re-render on zoom changes.
  createEffect(on(scale, () => void renderPages(), { defer: true }));
  // Repaint highlight overlays whenever the set changes.
  createEffect(on(highlights, repaint));
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

  function repaint() {
    for (const n of Object.keys(hlLayers)) {
      const layer = hlLayers[Number(n)];
      layer.innerHTML = "";
      for (const h of highlights().filter((x) => x.page === Number(n))) {
        for (const r of h.position.rects) {
          const div = document.createElement("div");
          div.className = "pdf-hl";
          div.style.left = `${r.left * scale()}px`;
          div.style.top = `${r.top * scale()}px`;
          div.style.width = `${r.width * scale()}px`;
          div.style.height = `${r.height * scale()}px`;
          div.style.background = COLOR_RGBA[h.color] ?? COLOR_RGBA.yellow;
          layer.appendChild(div);
        }
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
