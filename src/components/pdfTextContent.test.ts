import { afterEach, expect, it, vi } from "vitest";
import type { PDFPageProxy } from "pdfjs-dist";
import { pdfTextContent } from "./pdfTextContent";

const iterator = Object.getOwnPropertyDescriptor(ReadableStream.prototype, Symbol.asyncIterator)!;
afterEach(() => { Object.defineProperty(ReadableStream.prototype, Symbol.asyncIterator, iterator); });

it("keeps all glyphs, fonts and language when the engine has readers but no async stream iteration", async () => {
  Object.defineProperty(ReadableStream.prototype, Symbol.asyncIterator, { ...iterator, value: undefined });
  const chunks = [{ items: [{ str: "First " }], styles: { A: { fontFamily: "serif" } }, lang: null },
    { items: [{ str: "second" }], styles: { B: { fontFamily: "sans-serif" } }, lang: "en" }];
  const stream = new ReadableStream({ start(controller) { chunks.forEach(x => controller.enqueue(x)); controller.close(); } });
  const getTextContent = vi.fn(async () => {
    // pdf.js 5.5+ uses this API, which is absent in the native WebKitGTK probe.
    for await (const _chunk of stream) { /* extraction never reaches a chunk */ }
    return {};
  });
  const page = { getTextContent, streamTextContent: () => stream } as unknown as PDFPageProxy;
  const content = await pdfTextContent(page);
  expect(content).toEqual({ items: [{ str: "First " }, { str: "second" }],
    styles: { A: { fontFamily: "serif" }, B: { fontFamily: "sans-serif" } }, lang: "en" });
  expect(getTextContent).not.toHaveBeenCalled();
  expect(stream.locked).toBe(false);
});

it("propagates a stream read failure and releases its reader lock", async () => {
  Object.defineProperty(ReadableStream.prototype, Symbol.asyncIterator, { ...iterator, value: undefined });
  const stream = new ReadableStream({ start(controller) { controller.error(new Error("text decode failed")); } });
  const page = { streamTextContent: () => stream } as unknown as PDFPageProxy;
  await expect(pdfTextContent(page)).rejects.toThrow("text decode failed");
  expect(stream.locked).toBe(false);
});

it.each([false, true])("preserves upstream extraction when async iteration works or the page is XFA (%s)", async (isPureXfa) => {
  if (isPureXfa) Object.defineProperty(ReadableStream.prototype, Symbol.asyncIterator, { ...iterator, value: undefined });
  const content = { items: [], styles: {}, lang: "de" };
  const page = { isPureXfa, getTextContent: vi.fn().mockResolvedValue(content), streamTextContent: vi.fn() };
  expect(await pdfTextContent(page as unknown as PDFPageProxy)).toBe(content);
  expect(page.streamTextContent).not.toHaveBeenCalled();
});
