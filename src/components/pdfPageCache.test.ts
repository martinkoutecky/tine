import { expect, it, vi } from "vitest";
import type { PDFPageProxy } from "pdfjs-dist";
import { createPdfPageCache } from "./pdfPageCache";

function page() { return { cleanup: vi.fn(() => true) } as unknown as PDFPageProxy; }

it("counts each viewer once and cleans only when the last split releases", () => {
  const shared = page();
  const first = createPdfPageCache(), second = createPdfPageCache();
  first.retain(1, shared);
  first.retain(1, shared);
  second.retain(1, shared);
  first.release(1);
  first.release(1);
  expect(shared.cleanup).not.toHaveBeenCalled();
  second.clear();
  expect(shared.cleanup).toHaveBeenCalledOnce();
  second.clear();
  expect(shared.cleanup).toHaveBeenCalledOnce();
});

it("isolates equal page numbers in different documents and releases pages without a canvas", () => {
  const oldPage = page(), replacement = page(), otherDocument = page();
  const first = createPdfPageCache(), second = createPdfPageCache();
  first.retain(1, oldPage);
  second.retain(1, otherDocument);
  first.retain(1, replacement);
  expect(oldPage.cleanup).toHaveBeenCalledOnce();
  expect(otherDocument.cleanup).not.toHaveBeenCalled();
  first.clear();
  expect(replacement.cleanup).toHaveBeenCalledOnce();
  expect(otherDocument.cleanup).not.toHaveBeenCalled();
  second.clear();
  expect(otherDocument.cleanup).toHaveBeenCalledOnce();
});
