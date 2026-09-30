import * as pdfjs from "pdfjs-dist";
import { isMobilePlatform } from "../nativeChrome";

const TILE_CSS_SIZE = 768;
const TILE_PIXEL_CAP = 1_572_864;
const TILE_COUNT_CAP = isMobilePlatform ? 4 : 8;
/** Shared maximum tile backing pixels per reader (6 Mi mobile, 12 Mi desktop). */
export const PDF_TILE_PIXEL_BUDGET = TILE_COUNT_CAP * TILE_PIXEL_CAP;

interface Tile {
  key: string;
  page: number;
  canvas: HTMLCanvasElement;
  pixels: number;
  task?: pdfjs.RenderTask;
}

/** Render only the visible high-zoom page regions into bounded canvases.
 * At most TILE_COUNT_CAP tiles and PDF_TILE_PIXEL_BUDGET pixels are retained per
 * viewer; stale render tasks are canceled when a page or zoom is retired. */
export function createPdfTiles(onError: (error: unknown) => void) {
  const tiles = new Map<string, Tile>();
  type View = { page: pdfjs.PDFPageProxy; pageNumber: number; wrap: HTMLElement; scroller: HTMLElement; scale: number };
  type Wanted = { key: string; x: number; y: number; priority: number; view: View };
  const views = new Map<number, View>();
  let wanted: Wanted[] = [];
  let generation = 0;
  let pending = 0;

  function remove(tile: Tile) {
    tile.task?.cancel();
    tile.canvas.width = 0;
    tile.canvas.height = 0;
    tile.canvas.remove();
    tiles.delete(tile.key);
  }

  /** Release all transient tile rasters and invalidate pending jobs. */
  function reset() {
    generation++;
    pending = 0;
    views.clear();
    wanted = [];
    for (const tile of [...tiles.values()]) remove(tile);
  }

  /** Release a page that left the PDF canvas cache. */
  function releasePage(page: number) {
    views.delete(page);
    for (const tile of [...tiles.values()]) if (tile.page === page) remove(tile);
    updateWanted();
    pump();
  }

  function regions(view: View): Wanted[] {
    const { wrap, scroller, pageNumber, scale } = view;
    const pageRect = wrap.getBoundingClientRect();
    const scrollRect = scroller.getBoundingClientRect();
    const left = Math.max(0, scrollRect.left - pageRect.left);
    const top = Math.max(0, scrollRect.top - pageRect.top);
    const right = Math.min(pageRect.width, scrollRect.right - pageRect.left);
    const bottom = Math.min(pageRect.height, scrollRect.bottom - pageRect.top);
    if (right <= left || bottom <= top || !wrap.isConnected) return [];
    const startX = Math.floor(left / TILE_CSS_SIZE);
    const endX = Math.floor((right - 1) / TILE_CSS_SIZE);
    const startY = Math.floor(top / TILE_CSS_SIZE);
    const endY = Math.floor((bottom - 1) / TILE_CSS_SIZE);
    const wanted: Wanted[] = [];
    for (let y = startY; y <= endY; y++) for (let x = startX; x <= endX; x++) {
      const key = `${pageNumber}:${scale}:${x}:${y}`;
      const centerX = (x + 0.5) * TILE_CSS_SIZE;
      const centerY = (y + 0.5) * TILE_CSS_SIZE;
      wanted.push({ key, x, y, view, priority: Math.abs(centerX - (left + right) / 2) + Math.abs(centerY - (top + bottom) / 2) });
    }
    wanted.sort((a, b) => a.priority - b.priority);
    wanted.length = Math.min(wanted.length, TILE_COUNT_CAP);
    return wanted;
  }

  // One viewer-wide plan shares the cap fairly among visible pages. Completion
  // fills only this plan; it never reinstates an evicted page's private plan.
  function updateWanted() {
    const groups = [...views.values()].map(regions);
    wanted = [];
    for (let index = 0; index < TILE_COUNT_CAP && wanted.length < TILE_COUNT_CAP; index++) {
      for (const group of groups) {
        if (group[index]) wanted.push(group[index]);
        if (wanted.length === TILE_COUNT_CAP) break;
      }
    }
    const keys = new Set(wanted.map((item) => item.key));
    for (const tile of [...tiles.values()]) if (!keys.has(tile.key)) remove(tile);
  }

  /** Refresh all visible page regions; O(visible regions), bounded to the
   * viewer's shared canvas/pixel cap. Overlapping calls and completions share
   * one plan and at most two render jobs, so a stable viewport reaches rest. */
  function refresh(page: pdfjs.PDFPageProxy, pageNumber: number, wrap: HTMLElement,
    scroller: HTMLElement, scale: number) {
    views.set(pageNumber, { page, pageNumber, wrap, scroller, scale });
    updateWanted();
    pump();
  }

  function pump() {
    const current = generation;
    for (const item of wanted) {
      if (tiles.has(item.key)) continue;
      if (pending >= 2) break;
      const { page, pageNumber, wrap, scale } = item.view;
      if (!wrap.isConnected) continue;
      const viewport = page.getViewport({ scale });
      const layer = wrap.querySelector<HTMLElement>(".pdf-tile-layer") ?? document.createElement("div");
      if (!layer.isConnected) {
        layer.className = "pdf-tile-layer";
        wrap.insertBefore(layer, wrap.querySelector(".textLayer"));
      }
      const width = Math.min(TILE_CSS_SIZE, viewport.width - item.x * TILE_CSS_SIZE);
      const height = Math.min(TILE_CSS_SIZE, viewport.height - item.y * TILE_CSS_SIZE);
      if (width <= 0 || height <= 0) continue;
      const ratio = Math.min(window.devicePixelRatio || 1, 2, Math.sqrt(TILE_PIXEL_CAP / (width * height)));
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.floor(width * ratio));
      canvas.height = Math.max(1, Math.floor(height * ratio));
      canvas.style.left = `${item.x * TILE_CSS_SIZE}px`;
      canvas.style.top = `${item.y * TILE_CSS_SIZE}px`;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
      layer.appendChild(canvas);
      const tile: Tile = { key: item.key, page: pageNumber, canvas, pixels: canvas.width * canvas.height };
      tiles.set(item.key, tile);
      pending++;
      try {
        tile.task = page.render({ canvasContext: canvas.getContext("2d")!, viewport,
          transform: [ratio, 0, 0, ratio, -item.x * TILE_CSS_SIZE * ratio, -item.y * TILE_CSS_SIZE * ratio] });
        void tile.task.promise.catch((error: unknown) => {
          if ((error as { name?: string }).name !== "RenderingCancelledException" && generation === current) onError(error);
        }).finally(() => {
          if (generation !== current) return;
          pending--;
          pump();
        });
      } catch (error) {
        pending--;
        // Keep the failed slot until the next viewport change instead of
        // retrying a failing render forever from other jobs' completions.
        if (generation === current) onError(error);
      }
    }
  }

  return { refresh, reset, releasePage };
}
