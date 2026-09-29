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
    for (const tile of [...tiles.values()]) remove(tile);
  }

  /** Release a page that left the PDF canvas cache. */
  function releasePage(page: number) {
    for (const tile of [...tiles.values()]) if (tile.page === page) remove(tile);
  }

  /** Refresh visible regions at the current scroll and scale. Calls can overlap;
   * each tile has one canvas and one render task, with a global two-task cap. */
  function refresh(page: pdfjs.PDFPageProxy, pageNumber: number, wrap: HTMLElement,
    scroller: HTMLElement, scale: number) {
    const pageRect = wrap.getBoundingClientRect();
    const scrollRect = scroller.getBoundingClientRect();
    const left = Math.max(0, scrollRect.left - pageRect.left);
    const top = Math.max(0, scrollRect.top - pageRect.top);
    const right = Math.min(pageRect.width, scrollRect.right - pageRect.left);
    const bottom = Math.min(pageRect.height, scrollRect.bottom - pageRect.top);
    if (right <= left || bottom <= top) return;
    const startX = Math.floor(left / TILE_CSS_SIZE);
    const endX = Math.floor((right - 1) / TILE_CSS_SIZE);
    const startY = Math.floor(top / TILE_CSS_SIZE);
    const endY = Math.floor((bottom - 1) / TILE_CSS_SIZE);
    const wanted: Array<{ key: string; x: number; y: number; priority: number }> = [];
    for (let y = startY; y <= endY; y++) for (let x = startX; x <= endX; x++) {
      const key = `${pageNumber}:${scale}:${x}:${y}`;
      const centerX = (x + 0.5) * TILE_CSS_SIZE;
      const centerY = (y + 0.5) * TILE_CSS_SIZE;
      wanted.push({ key, x, y, priority: Math.abs(centerX - (left + right) / 2) + Math.abs(centerY - (top + bottom) / 2) });
    }
    wanted.sort((a, b) => a.priority - b.priority);
    wanted.length = Math.min(wanted.length, TILE_COUNT_CAP);
    const wantedKeys = new Set(wanted.map((item) => item.key));
    for (const tile of [...tiles.values()]) {
      if (tile.page === pageNumber && !wantedKeys.has(tile.key)) remove(tile);
    }
    const layer = wrap.querySelector<HTMLElement>(".pdf-tile-layer") ?? document.createElement("div");
    if (!layer.isConnected) {
      layer.className = "pdf-tile-layer";
      wrap.insertBefore(layer, wrap.querySelector(".textLayer"));
    }
    const current = generation;
    const viewport = page.getViewport({ scale });
    for (const item of wanted) {
      if (tiles.has(item.key)) continue;
      if (pending >= 2) break;
      // Keep the central visible region crisp first when viewport exceeds cap.
      while (tiles.size >= TILE_COUNT_CAP || [...tiles.values()].reduce((sum, tile) => sum + tile.pixels, 0) >= PDF_TILE_PIXEL_BUDGET) {
        const oldest = tiles.values().next().value as Tile | undefined;
        if (!oldest) break;
        remove(oldest);
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
          if (wrap.isConnected) refresh(page, pageNumber, wrap, scroller, scale);
        });
      } catch (error) {
        pending--;
        remove(tile);
        if (generation === current) onError(error);
      }
    }
  }

  return { refresh, reset, releasePage };
}
