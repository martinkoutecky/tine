import * as pdfjs from "pdfjs-dist";
import type { PdfOwnership } from "../pdfOwnership";
import type { Rect } from "../types";
import { isMobilePlatform } from "../nativeChrome";
import { PDF_THEMES, type PdfTheme } from "./pdfViewerPalette";

export const PDF_THEME_KEY = "ls-pdf-viewer-theme";

/** Destroying a viewer document only releases transient pdf.js resources.
 * A rejection cannot affect graph content; browser teardown reclaims them. */
function ignorePdfDestroyFailure(_error: unknown): void {}
export function discardPdfDocument(doc: pdfjs.PDFDocumentProxy): void { void doc.destroy().then(undefined, ignorePdfDestroyFailure); }

export function storedPdfTheme(): PdfTheme {
  try {
    const stored = window.localStorage.getItem(PDF_THEME_KEY);
    return PDF_THEMES.includes(stored as PdfTheme) ? stored as PdfTheme : "light";
  } catch {
    return "light";
  }
}

export function isPdfPageRef(value: unknown): value is { num: number; gen: number } {
  if (!value || typeof value !== "object") return false;
  const ref = value as { num?: unknown; gen?: unknown };
  return Number.isSafeInteger(ref.num) && Number(ref.num) >= 0 &&
    Number.isSafeInteger(ref.gen) && Number(ref.gen) >= 0;
}

// Resource ceilings are deliberately generous for books, scanned documents, and
// architectural drawings, but bounded below the point where pdf.js/WebView canvas
// allocations can take down the whole application.
export const MAX_PDF_BYTES = 256 * 1024 * 1024;
export const MAX_PDF_PAGES = 5000;
export const MAX_PAGE_DIMENSION = 14_400; // PDF points: 200 inches at 72 dpi.
export const MAX_CANVAS_DIMENSION = 16_384;
export const MAX_CANVAS_PIXELS = isMobilePlatform ? 8_388_608 : 16_777_216;
// Canvas backing stores are normally 4-byte RGBA. Bound the aggregate rather
// than counting pages: at high zoom one page can be far larger than 24 ordinary
// fit-width pages. Mobile keeps at most ~64 MiB; desktop ~192 MiB.
export const PDF_CANVAS_CACHE_PIXEL_BUDGET = isMobilePlatform ? 16_777_216 : 50_331_648;
export const PDF_CANVAS_CACHE_PAGE_CAP = isMobilePlatform ? 6 : 12;

export function isPdfAreaModifier(
  event: Pick<MouseEvent, "metaKey" | "shiftKey">,
  mac: boolean
): boolean {
  return mac ? event.metaKey : event.shiftKey;
}

export interface PendingArea {
  page: number;
  wrap: HTMLElement;
  rect: Rect;
}

export interface PdfTarget {
  filename: string;
  label: string;
  owner: PdfOwnership;
  page?: number;
  scale?: number;
  highlightId?: string;
}

export function errorMessage(action: string, err?: unknown): string {
  const detail = err instanceof Error ? err.message : err ? String(err) : "";
  return detail ? `${action}: ${detail}` : action;
}

export function pageDimensionsError(page: number, width: number, height: number): string | null {
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    return `PDF page ${page} reports invalid dimensions (${width} x ${height}).`;
  }
  if (width > MAX_PAGE_DIMENSION || height > MAX_PAGE_DIMENSION) {
    return `PDF page ${page} is too large to render safely (${Math.round(width)} x ${Math.round(height)} points).`;
  }
  return null;
}

export function safeCanvasSize(width: number, height: number, maxPixels = MAX_CANVAS_PIXELS) {
  const pixelLimit = Math.max(1, Math.min(MAX_CANVAS_PIXELS, maxPixels));
  const requestedRatio = Math.min(window.devicePixelRatio || 1, 2);
  const ratio = Math.min(
    requestedRatio,
    MAX_CANVAS_DIMENSION / width,
    MAX_CANVAS_DIMENSION / height,
    Math.sqrt(pixelLimit / (width * height))
  );
  if (!Number.isFinite(ratio) || ratio <= 0) return null;
  return {
    ratio,
    width: Math.max(1, Math.min(MAX_CANVAS_DIMENSION, Math.floor(width * ratio))),
    height: Math.max(1, Math.min(MAX_CANVAS_DIMENSION, Math.floor(height * ratio))),
  };
}


/** Crop a rendered region using its actual backing ratio; output is one PNG. */
export async function cropPdfCanvas(canvas: HTMLCanvasElement, rect: Rect, scale: number, pixelRatio: number): Promise<Uint8Array | null> {
    // Map unscaled coordinates to backing pixels.
    const dpr = pixelRatio;
    const f = scale * dpr;
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

/** Restore a page if needed before capturing an area annotation. */
export async function cropPdfArea(page: number, wrap: HTMLElement, rect: Rect,
  scale: number, renderedScale: number | undefined, pixelRatio: number,
  renderPage: (page: number) => Promise<void>): Promise<Uint8Array | null> {
  let canvas = wrap.querySelector("canvas") as HTMLCanvasElement | null;
  if (!canvas || renderedScale !== scale) {
    await renderPage(page);
    canvas = wrap.querySelector("canvas") as HTMLCanvasElement | null;
  }
  return canvas ? cropPdfCanvas(canvas, rect, scale, pixelRatio) : null;
}
