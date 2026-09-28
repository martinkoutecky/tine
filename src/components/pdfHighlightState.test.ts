import { describe, expect, it } from "vitest";
import type { Highlight } from "../types";
import { rebasePdfHighlights, unusedPdfCrops } from "./pdfHighlightState";

const rect = { left: 2, top: 4, width: 6, height: 8 };
function highlight(id: string, color = "yellow", image: number | null = null): Highlight {
  return { id, page: 1, position: { page: 1, bounding: rect, rects: [rect] }, color, text: "text", image };
}

describe("PDF highlight state", () => {
  it("rebases changed fields and local deletion while preserving disk-only additions", () => {
    const a = highlight("a");
    const b = highlight("b");
    const diskOnly = highlight("disk");
    expect(rebasePdfHighlights([a, b], [{ ...a, color: "green" }], [
      { ...a, text: "disk text" }, { ...b, color: "blue" }, diskOnly,
    ])).toEqual([{ ...a, color: "green", text: "disk text" }, diskOnly]);
  });

  it("keeps a locally changed highlight when disk deleted its baseline", () => {
    const original = highlight("a");
    expect(rebasePdfHighlights([original], [{ ...original, color: "green" }], []))
      .toEqual([{ ...original, color: "green" }]);
    expect(rebasePdfHighlights([original], [original], [])).toEqual([]);
  });

  it("retires a crop only after neither committed nor optimistic highlights use it", () => {
    const crops = new Map([["a", { page: 1, stamp: 42 }]]);
    const area = highlight("a", "yellow", 42);
    expect(unusedPdfCrops(crops, [], [area])).toEqual([]);
    expect(unusedPdfCrops(crops, [area], [])).toEqual([]);
    expect(unusedPdfCrops(crops, [], [])).toEqual([["a", { page: 1, stamp: 42 }]]);
  });
});
