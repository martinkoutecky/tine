import { pdf_asset_key } from "./render/wasm/lsdoc_wasm.js";
// PDF geometry and presentation; resource identity comes from the Rust door.

import type { Highlight, Rect } from "./types";

export interface PdfPageDimensions {
  w: number;
  h: number;
}

function hasSourceSpace(rect: Rect): rect is Rect & { source_width: number; source_height: number } {
  return Number.isFinite(rect.source_width) && Number.isFinite(rect.source_height) &&
    rect.source_width! > 0 && rect.source_height! > 0;
}

/** Convert a persisted rectangle to pdf.js scale-1 page coordinates. Current
 * Logseq stores coordinates relative to the viewport size at creation time;
 * older Tine rectangles are already in page coordinates. */
export function rectInPageSpace(rect: Rect, page: PdfPageDimensions): Rect {
  if (!hasSourceSpace(rect)) return rect;
  const x = page.w / rect.source_width;
  const y = page.h / rect.source_height;
  return {
    ...rect,
    left: rect.left * x,
    top: rect.top * y,
    width: rect.width * x,
    height: rect.height * y,
  };
}

/** Attach the coordinate-space dimensions required by Logseq's sidecar shape.
 * Used before persistence to migrate old Tine rectangles without changing their
 * visible scale-1 page coordinates. */
export function rectWithSourceSpace(rect: Rect, page: PdfPageDimensions): Rect {
  if (hasSourceSpace(rect)) return rect;
  return { ...rect, source_width: page.w, source_height: page.h };
}

/** OG stores an area highlight entirely in `bounding`; `rects` is reserved for
 * the line fragments of a text selection and is empty for a captured region. */
export function areaHighlightPosition(page: number, bounding: Rect): Highlight["position"] {
  return { page, bounding, rects: [] };
}

/** Live preview identity; sanitization is owned by the native pdf_key door. */
export function assetKey(filename: string): string {
  return pdf_asset_key(filename, true);
}

export function hlsPageName(filename: string): string {
  return `hls__${assetKey(filename)}`;
}

export const HL_COLOR_BG: Record<string, string> = {
  yellow: "rgba(255, 226, 86, 0.45)",
  green: "rgba(116, 226, 130, 0.45)",
  blue: "rgba(110, 176, 246, 0.45)",
  red: "rgba(246, 130, 130, 0.45)",
  purple: "rgba(190, 140, 246, 0.45)",
};

// Solid swatch colors for the highlight dot in the note bullet (matches OG's
// colored-dot prefix on annotation blocks).
export const HL_COLOR_SOLID: Record<string, string> = {
  yellow: "#f5c518",
  green: "#3fbf57",
  blue: "#4a9eff",
  red: "#ec5c5c",
  purple: "#a86ff0",
};

export const HL_COLORS = ["yellow", "green", "blue", "red", "purple"];
