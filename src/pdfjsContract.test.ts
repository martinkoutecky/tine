import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const packageJson = JSON.parse(readFileSync("package.json", "utf8")) as {
  dependencies?: Record<string, string>;
};
const packageLock = JSON.parse(readFileSync("package-lock.json", "utf8")) as {
  packages?: Record<string, { version?: string; dependencies?: Record<string, string> }>;
};
const viewerSource = readFileSync("node_modules/pdfjs-dist/web/pdf_viewer.mjs", "utf8");
const viewerTypes = readFileSync(
  "node_modules/pdfjs-dist/types/web/pdf_viewer.component.d.ts",
  "utf8",
);
const pageViewTypes = readFileSync(
  "node_modules/pdfjs-dist/types/web/pdf_page_view.d.ts",
  "utf8",
);
const apiSource = readFileSync("node_modules/pdfjs-dist/build/pdf.mjs", "utf8");

describe("pinned PDF.js viewer contract", () => {
  it("pins the API and web viewer to the exact characterized version", () => {
    expect(packageJson.dependencies?.["pdfjs-dist"]).toBe("4.10.38");
    expect(packageLock.packages?.[""]?.dependencies?.["pdfjs-dist"]).toBe("4.10.38");
    expect(packageLock.packages?.["node_modules/pdfjs-dist"]?.version).toBe("4.10.38");
    expect(viewerSource).toContain('const viewerVersion = "4.10.38"');
  });

  it("records the public lifecycle surface instead of importing private queues", () => {
    expect(viewerTypes).toMatch(/\bPDFViewer\b/);
    expect(viewerTypes).toMatch(/\bPDFPageView\b/);
    expect(viewerTypes).not.toMatch(/\bPDFRenderingQueue\b/);
    expect(viewerTypes).not.toMatch(/\bPDFPageViewBuffer\b/);
  });

  it("records the full-page canvas and count-buffer behavior that tiling must replace", () => {
    expect(viewerSource).toContain("const canvas = document.createElement(\"canvas\")");
    expect(viewerSource).toContain("this.canvas = canvas");
    expect(viewerSource).toContain("const DEFAULT_CACHE_SIZE = 10");
    expect(viewerSource).toContain("Math.max(DEFAULT_CACHE_SIZE, 2 * numVisiblePages + 1)");
    expect(viewerSource).toContain("onlyCssZoom");
  });

  it("records integration constraints that every pane-owned viewer must satisfy", () => {
    expect(viewerSource).toContain("The `container` must be absolutely positioned.");
    expect(viewerSource).toContain('id = "hiddenCopyElement"');
    expect(viewerSource).toContain("getCachedPageViews()");
  });

  it("pins the destructive page-view lifecycle that Tine must not call", () => {
    expect(pageViewTypes).toMatch(/\bdestroy\(\): void/);
    expect(viewerSource).toMatch(/destroy\(\) \{\s*this\.reset\(\);\s*this\.pdfPage\?\.cleanup\(\);/);
  });

  it("pins the private count-buffer and lazy document-loading seams", () => {
    expect(viewerSource).toContain("#buffer = null");
    expect(viewerSource).toMatch(/getCachedPageViews\(\) \{\s*return new Set\(this\.#buffer\);/);
    expect(viewerSource).toContain("pdfDocument.loadingParams.disableAutoFetch");
    expect(apiSource).toContain("const disableAutoFetch = src.disableAutoFetch === true");
    expect(apiSource).toMatch(/loadingParams: \{\s*disableAutoFetch,/);
  });
});
