import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync("src/pdfRenderer.ts", "utf8");

describe("direct PDFPageView ownership boundary", () => {
  it("uses the exported page view directly instead of the full viewer", () => {
    expect(source).toContain("new PDFPageView(");
    expect(source).not.toMatch(/\bnew PDFViewer\b/);
  });

  it("does not invoke destructive shared-proxy lifecycle methods", () => {
    expect(source).not.toMatch(/\.destroy\s*\(/);
    expect(source).not.toMatch(/\.cleanup\s*\(/);
  });
});
