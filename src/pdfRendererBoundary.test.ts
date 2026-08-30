import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync("src/pdfRenderer.ts", "utf8");
const componentSource = readFileSync("src/components/PdfViewer.tsx", "utf8");

describe("direct PDFPageView ownership boundary", () => {
  it("uses the exported page view directly instead of the full viewer", () => {
    expect(source).toContain("new PDFPageView(");
    expect(source).not.toMatch(/\bnew PDFViewer\b/);
  });

  it("does not invoke destructive shared-proxy lifecycle methods", () => {
    expect(source).not.toMatch(/\.destroy\s*\(/);
    expect(source).not.toMatch(/\.cleanup\s*\(/);
  });

  it("keeps the component behind the adapter instead of rebuilding PDF.js internals", () => {
    expect(componentSource).toContain("new PdfPageViewRenderer(");
    expect(componentSource).not.toMatch(/\bpage\.render\s*\(/);
    expect(componentSource).not.toContain("new (pdfjs as any).TextLayer");
    expect(componentSource).not.toContain("renderQueue");
  });
});
