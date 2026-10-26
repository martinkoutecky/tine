import type { Rect } from "../types";
import type { PdfPageDimensions } from "../pdf";
import { elementFromPointIn, isElementNode, selectionOf } from "../windowRealm";

export interface PdfTextSelection {
  page: number;
  rects: Rect[];
  bounding: Rect;
  text: string;
  menu: { x: number; y: number };
}

/** Convert a browser text selection to PDF page-space coordinates. Returns
 * null for empty, off-page, or dimensionless selections. Cost O(selected rects). */
export function pdfTextSelection(
  target: EventTarget | null,
  anchor: { x: number; y: number } | undefined,
  dims: readonly PdfPageDimensions[],
  scale: number,
): PdfTextSelection | null {
  const selection = selectionOf(target);
  if (!selection || selection.isCollapsed || !selection.toString().trim()) return null;
  const range = selection.getRangeAt(0);
  const clientRects = Array.from(range.getClientRects()).filter((r) => r.width > 0 && r.height > 0);
  if (!clientRects.length) return null;
  const first = clientRects[0];
  const common = range.commonAncestorContainer;
  const commonElement = isElementNode(common) ? common : common?.parentElement;
  const targetElement = isElementNode(target) ? target : null;
  const wrap = (commonElement?.closest(".pdf-page") ?? targetElement?.closest(".pdf-page")
    ?? elementFromPointIn(common, first.left, first.top)?.closest(".pdf-page")) as HTMLElement | null;
  if (!wrap) return null;
  const page = Number(wrap.dataset.page);
  if (!Number.isSafeInteger(page) || !dims[page]) return null;
  const base = wrap.getBoundingClientRect();
  const rects: Rect[] = clientRects.map((r) => ({
    left: (r.left - base.left) / scale,
    top: (r.top - base.top) / scale,
    width: r.width / scale,
    height: r.height / scale,
    source_width: dims[page].w,
    source_height: dims[page].h,
  }));
  const left = Math.min(...rects.map((r) => r.left));
  const top = Math.min(...rects.map((r) => r.top));
  const right = Math.max(...rects.map((r) => r.left + r.width));
  const bottom = Math.max(...rects.map((r) => r.top + r.height));
  const last = clientRects[clientRects.length - 1];
  return { page, rects, bounding: {
    left, top, width: right - left, height: bottom - top,
    source_width: dims[page].w, source_height: dims[page].h,
  }, text: selection.toString(), menu: { x: anchor?.x ?? last.right, y: anchor?.y ?? last.bottom } };
}
