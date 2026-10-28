import type { PDFPageProxy } from "pdfjs-dist";

// A PDFDocumentProxy (and therefore its page proxies) is shared by splits.
// Retiring one view must not discard another view's cached decoded data.
const holders = new WeakMap<PDFPageProxy, number>();

/** One viewer's retained pages. Like pdf.js PDFPageView.destroy (4.10.38,
 * web/pdf_viewer.mjs), eviction cleans decoded images and operator lists after
 * cancelling render/text/tile work. pdf.js defers cleanup until cancelled
 * renders settle. Unlike its single-view cache, shared splits release only
 * their own hold; the last holder requests cleanup. No graph reads or writes. */
export function createPdfPageCache() {
  const pages: Record<number, PDFPageProxy> = {};
  function release(number: number) {
    const page = pages[number];
    if (!page) return;
    delete pages[number];
    const remaining = (holders.get(page) ?? 1) - 1;
    if (remaining > 0) holders.set(page, remaining);
    else {
      holders.delete(page);
      page.cleanup();
    }
  }
  function retain(number: number, page: PDFPageProxy) {
    if (pages[number] === page) return;
    release(number);
    pages[number] = page;
    holders.set(page, (holders.get(page) ?? 0) + 1);
  }
  function clear() { for (const number of Object.keys(pages)) release(Number(number)); }
  return { pages, retain, release, clear };
}
