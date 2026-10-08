import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { createPdfFind } from "./pdfFind";
import { setToasts, toasts } from "../toasts";

// UI-OG-C5-P6-FIND: PDF Find reports a page pdf.js cannot read, and a
// retired Find (closed, re-queried, torn down) never lands its pending
// navigation (I-9, I-20, I-21).

function pdfWith(texts: (string | Error)[]) {
  return {
    numPages: texts.length,
    getPage: vi.fn(async (n: number) => {
      const text = texts[n - 1];
      if (text instanceof Error) throw text;
      return { getTextContent: async () => ({ items: [{ str: text }] }) };
    }),
  } as any;
}

function setup(doc: ReturnType<typeof pdfWith>, over: Record<string, unknown> = {}) {
  const calls = { scroll: [] as number[] };
  const ctx = {
    document: () => doc,
    owner: () => true,
    requests: {},
    scrollToPage: (page: number) => { calls.scroll.push(page); },
    renderPage: async () => {},
    renderedScale: () => 1,
    textScale: () => 1,
    buildTextLayer: async () => {},
    textLayer: () => undefined,
    scrollElement: () => document.createElement("div"),
    scale: () => 1,
    ...over,
  };
  let find!: ReturnType<typeof createPdfFind>;
  const dispose = createRoot((d) => { find = createPdfFind(ctx as any); return d; });
  return { find, calls, dispose };
}

async function settle() { for (let i = 0; i < 30; i++) await Promise.resolve(); }

describe("PdfFind failures and retirement", () => {
  beforeEach(() => { vi.useFakeTimers(); setToasts([]); });
  afterEach(() => { vi.useRealTimers(); setToasts([]); });

  it("reports an unreadable page and clears the previous query's results", async () => {
    let doc = pdfWith(["alpha beta", "alpha"]);
    const { find, dispose } = setup(doc, { document: () => doc });
    try {
      find.openFind();
      find.scheduleFind("alpha");
      await vi.advanceTimersByTimeAsync(200);
      await settle();
      expect(find.findCount()).toBe(2);
      // The viewer now holds a document whose uncached page 3 cannot be read.
      doc = pdfWith(["alpha beta", "alpha", new Error("bad page")]);
      find.scheduleFind("alph");
      await vi.advanceTimersByTimeAsync(200);
      await settle();
      expect(toasts().some((t) => t.message === "Couldn't search this PDF. The search results were cleared.")).toBe(true);
      expect(find.findCount(), "stale results must not answer the new query").toBe(0);
      expect(find.findCur()).toBe(0);
    } finally { dispose(); }
  });

  it("does not land a scan's navigation after Find was closed", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const doc = pdfWith(["alpha", "alpha"]);
    const first = doc.getPage.getMockImplementation()!;
    doc.getPage.mockImplementation(async (n: number) => { if (n === 2) await gate; return first(n); });
    const { find, calls, dispose } = setup(doc);
    try {
      find.openFind();
      find.scheduleFind("alpha");
      await vi.advanceTimersByTimeAsync(200);
      await settle();
      find.closeFind();
      release();
      await settle();
      expect(calls.scroll, "a retired scan must not scroll the reader").toEqual([]);
      expect(find.findCount()).toBe(0);
    } finally { dispose(); }
  });

  it("drops a pending debounced scan when Find closes", async () => {
    const doc = pdfWith(["alpha"]);
    const { find, calls, dispose } = setup(doc);
    try {
      find.openFind();
      find.scheduleFind("alpha");
      find.closeFind();
      await vi.advanceTimersByTimeAsync(400);
      await settle();
      expect(doc.getPage).not.toHaveBeenCalled();
      expect(calls.scroll).toEqual([]);
    } finally { dispose(); }
  });

  it("lets a navigation that is waiting on a render finish without a TypeError when the query is cleared", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const doc = pdfWith(["alpha", "alpha"]);
    const { find, dispose } = setup(doc, { renderedScale: () => 2, renderPage: () => gate });
    try {
      find.openFind();
      find.scheduleFind("alpha");
      await vi.advanceTimersByTimeAsync(200);
      await settle();
      expect(find.findCount()).toBe(2);
      find.scheduleFind("");
      await vi.advanceTimersByTimeAsync(200);
      await settle();
      release();
      await settle();
      expect(find.findCount()).toBe(0);
      expect(toasts().some((t) => t.message.includes("Couldn't search"))).toBe(false);
    } finally { dispose(); }
  });
});
