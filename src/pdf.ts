// Shared PDF helpers (mirror the Rust asset_key / hls naming and colors).

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

// MUST stay byte-for-byte in sync with the Rust `asset_key` (crates/tine-core/
// src/pdf.rs) — the frontend derives the hls__ page name the Notes pane opens,
// and the backend derives the page it writes; a divergence points them at
// different pages. Matches OG's `sanitize-filename`: strip only OS-illegal
// characters, preserve case / `-` / `_` / spaces.
export function assetKey(filename: string): string {
  const stem = filename.replace(/\.(pdf)$/i, "");
  let out = Array.from(stem)
    .filter((c) => {
      const n = c.codePointAt(0)!;
      if (n <= 0x1f || (n >= 0x80 && n <= 0x9f)) return false; // control chars
      return !'/?<>\\:*|"'.includes(c); // reserved/illegal set
    })
    .join("")
    .replace(/[. ]+$/, ""); // trailing dots/spaces (Windows)
  // Windows reserved device names (optionally with an extension) → removed.
  const base = (out.split(".")[0] ?? "").toLowerCase();
  if (/^(con|prn|aux|nul|com[0-9]|lpt[0-9])$/.test(base)) out = "";
  return out;
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
