import type { PDFPageProxy } from "pdfjs-dist";

/** pdf.js 5.5 uses async stream iteration in getTextContent. WebKitGTK can
 * provide getReader without that newer API, even with pdf.js's legacy build.
 * Aggregate the same chunks through its reader there; leave XFA to pdf.js. */
export async function pdfTextContent(page: PDFPageProxy): Promise<Awaited<ReturnType<PDFPageProxy["getTextContent"]>>> {
  if (page.isPureXfa || typeof ReadableStream.prototype[Symbol.asyncIterator] === "function") {
    return page.getTextContent();
  }
  const reader = page.streamTextContent().getReader();
  const content: Awaited<ReturnType<PDFPageProxy["getTextContent"]>> = {
    items: [], styles: Object.create(null), lang: null,
  };
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      content.lang ??= value.lang;
      Object.assign(content.styles, value.styles);
      content.items.push(...value.items);
    }
  } finally { reader.releaseLock(); }
  return content;
}
