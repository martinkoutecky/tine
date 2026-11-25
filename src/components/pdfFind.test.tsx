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

// UI-GH1037-PDF-FIND-KEEPS-TYPING: a search result arriving while the user types
// a second query must not take the keyboard from the find box. Moving the
// document selection into the text layer did (#1037): later keystrokes no
// longer reached the box. The current match is painted as a CSS highlight.
describe("PdfFind keeps the find box's keyboard while matches land", () => {
  class FakeHighlight { ranges: Range[]; constructor(...ranges: Range[]) { this.ranges = ranges; } }
  const realm = window as unknown as { CSS?: { highlights?: Map<string, unknown> }; Highlight?: unknown };
  let savedCss: unknown, savedHighlight: unknown, registry: Map<string, unknown>;
  beforeEach(() => {
    vi.useFakeTimers();
    savedCss = realm.CSS; savedHighlight = realm.Highlight;
    registry = new Map();
    realm.CSS = { ...(savedCss as object), highlights: registry };
    realm.Highlight = FakeHighlight;
    if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
  });
  afterEach(() => {
    vi.useRealTimers();
    realm.CSS = savedCss as typeof realm.CSS; realm.Highlight = savedHighlight;
    document.body.replaceChildren();
    document.getSelection()?.removeAllRanges();
  });
  const painted = () => (registry.get("tine-pdf-find") as FakeHighlight | undefined)?.ranges.map(String) ?? [];

  it("leaves the selection in the find box and paints the match instead", async () => {
    const doc = pdfWith(["Tine PDF viewer", "Highlights persist to a page"]);
    const layers = [1, 2].map((n) => {
      const layer = document.createElement("div");
      for (const part of (n === 1 ? ["Tine PDF ", "viewer"] : ["Highlights ", "persist to a page"])) {
        const span = document.createElement("span");
        span.textContent = part;
        layer.appendChild(span);
      }
      document.body.appendChild(layer);
      return layer;
    });
    const scroller = document.createElement("div");
    document.body.appendChild(scroller);
    const input = document.createElement("input");
    document.body.appendChild(input);
    const { find, dispose } = setup(doc, { textLayer: (page: number) => layers[page - 1], scrollElement: () => scroller });
    find.setInput(input);
    const type = async (value: string) => {
      input.value = value;
      input.setSelectionRange(value.length, value.length);
      find.scheduleFind(value);
      await vi.advanceTimersByTimeAsync(200);
      await settle();
    };
    const keyboardStaysInBox = () => {
      const selection = document.getSelection()!;
      const node = selection.rangeCount ? selection.getRangeAt(0).startContainer : null;
      expect(layers.some((layer) => node && layer.contains(node)), "the document selection moved into the text layer").toBe(false);
      expect(document.activeElement).toBe(input);
    };
    try {
      find.openFind();
      await settle();
      input.focus();
      // First query lands; then the reporter selects it and types a new one.
      await type("viewer");
      expect(find.findCount()).toBe(1);
      keyboardStaysInBox();
      expect(painted()).toEqual(["viewer"]);
      input.select();
      await type("p");
      expect(find.findCount()).toBe(3);
      keyboardStaysInBox();
      expect(painted()).toEqual(["P"]);
      await type("persist");
      keyboardStaysInBox();
      expect(input.value).toBe("persist");
      expect(painted()).toEqual(["persist"]);
      // Enter between matches paints the next one and still leaves the box alone.
      await type("i");
      find.nextMatch(1);
      await settle();
      expect(find.findCur()).toBe(2);
      keyboardStaysInBox();
      // An emptied query and a closed Find both clear the paint.
      await type("");
      expect(painted()).toEqual([]);
      await type("viewer");
      expect(painted()).toEqual(["viewer"]);
      find.closeFind();
      expect(painted()).toEqual([]);
    } finally { dispose(); }
  });

  it("marks the match with the selection where the runtime has no CSS highlights (the old mark)", async () => {
    realm.CSS = savedCss as typeof realm.CSS; realm.Highlight = undefined;
    const doc = pdfWith(["Tine PDF viewer"]);
    const layer = document.createElement("div");
    const span = document.createElement("span");
    span.textContent = "Tine PDF viewer";
    layer.appendChild(span);
    document.body.appendChild(layer);
    const scroller = document.createElement("div");
    document.body.appendChild(scroller);
    const input = document.createElement("input");
    document.body.appendChild(input);
    const { find, dispose } = setup(doc, { textLayer: () => layer, scrollElement: () => scroller });
    find.setInput(input);
    const type = async (value: string) => {
      find.scheduleFind(value);
      await vi.advanceTimersByTimeAsync(200);
      await settle();
    };
    try {
      find.openFind();
      await settle();
      await type("viewer");
      expect(find.findCount()).toBe(1);
      expect(document.getSelection()!.toString()).toBe("viewer");
      await type("");
      expect(document.getSelection()!.toString()).toBe("");
      // A viewer that unmounts leaves the window's selection alone: it may be
      // another reader's match or the user's own selection by now.
      await type("viewer");
      dispose();
      expect(document.getSelection()!.toString()).toBe("viewer");
    } finally { dispose(); }
  });

  it("does not erase the match another reader painted in the same window", async () => {
    const doc = pdfWith(["alpha"]);
    const layer = document.createElement("div");
    layer.textContent = "alpha";
    document.body.appendChild(layer);
    const first = setup(doc, { textLayer: () => layer });
    const second = setup(doc, { textLayer: () => layer });
    try {
      first.find.openFind();
      first.find.scheduleFind("alpha");
      await vi.advanceTimersByTimeAsync(200);
      await settle();
      const firstPaint = registry.get("tine-pdf-find");
      second.find.openFind();
      second.find.scheduleFind("alp");
      await vi.advanceTimersByTimeAsync(200);
      await settle();
      expect(painted()).toEqual(["alp"]);
      first.find.closeFind();
      expect(painted(), "the first reader's close must not erase the second reader's match").toEqual(["alp"]);
      expect(registry.get("tine-pdf-find")).not.toBe(firstPaint);
      second.dispose();
      expect(painted(), "a torn-down reader lets its paint go").toEqual([]);
    } finally { first.dispose(); second.dispose(); }
  });
});
