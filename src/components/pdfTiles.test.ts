// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as pdfjs from "pdfjs-dist";
import { createPdfTiles } from "./pdfTiles";

function rect(left: number, top: number, width: number, height: number): DOMRect {
  return { left, top, width, height, right: left + width, bottom: top + height,
    x: left, y: top, toJSON: () => ({}) } as DOMRect;
}

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

describe("high zoom PDF tiles", () => {
  it("rasterizes visible regions, reuses them, and releases old regions on scroll", async () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const render = vi.fn(() => ({ promise: Promise.resolve(), cancel: vi.fn() }));
    const page = { getViewport: () => ({ width: 4096, height: 4096 }), render } as unknown as pdfjs.PDFPageProxy;
    const scroll = document.createElement("div");
    const wrap = document.createElement("div");
    const textLayer = document.createElement("div");
    textLayer.className = "textLayer";
    wrap.appendChild(textLayer);
    scroll.appendChild(wrap);
    document.body.appendChild(scroll);
    let left = 0;
    vi.spyOn(scroll, "getBoundingClientRect").mockImplementation(() => rect(left, 0, 1000, 1000));
    vi.spyOn(wrap, "getBoundingClientRect").mockImplementation(() => rect(0, 0, 4096, 4096));
    const tiles = createPdfTiles(() => { throw new Error("tile render failed"); });
    tiles.refresh(page, 1, wrap, scroll, 4);
    await vi.waitFor(() => expect(wrap.querySelectorAll(".pdf-tile-layer canvas").length).toBe(4));
    const first = wrap.querySelector(".pdf-tile-layer canvas")!;
    expect(render).toHaveBeenCalled();
    expect(render.mock.calls[0]).toBeDefined();
    tiles.refresh(page, 1, wrap, scroll, 4);
    expect(wrap.querySelectorAll(".pdf-tile-layer canvas").length).toBe(4);
    left = 1600;
    tiles.refresh(page, 1, wrap, scroll, 4);
    await vi.waitFor(() => expect(wrap.querySelectorAll(".pdf-tile-layer canvas").length).toBe(4));
    expect(first.isConnected).toBe(false);
    tiles.reset();
    expect(wrap.querySelectorAll(".pdf-tile-layer canvas").length).toBe(0);
  });

  it("bounds a large viewport to eight canvases and 12 Mi pixels", () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const page = { getViewport: () => ({ width: 16_000, height: 16_000 }),
      render: () => ({ promise: new Promise<void>(() => {}), cancel: vi.fn() }) } as unknown as pdfjs.PDFPageProxy;
    const scroll = document.createElement("div");
    const wrap = document.createElement("div");
    scroll.appendChild(wrap);
    document.body.appendChild(scroll);
    vi.spyOn(scroll, "getBoundingClientRect").mockReturnValue(rect(0, 0, 10_000, 10_000));
    vi.spyOn(wrap, "getBoundingClientRect").mockReturnValue(rect(0, 0, 16_000, 16_000));
    const tiles = createPdfTiles(() => {});
    tiles.refresh(page, 1, wrap, scroll, 4);
    const canvases = [...wrap.querySelectorAll("canvas")];
    expect(canvases.length).toBeLessThanOrEqual(8);
    expect(canvases.reduce((sum, canvas) => sum + canvas.width * canvas.height, 0)).toBeLessThanOrEqual(8 * 1_572_864);
    tiles.reset();
  });

  it("admits new zoom tiles after canceling unresolved old tasks", () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const render = vi.fn(() => ({ promise: new Promise<void>(() => {}), cancel: vi.fn() }));
    const page = { getViewport: () => ({ width: 4096, height: 4096 }), render } as unknown as pdfjs.PDFPageProxy;
    const scroll = document.createElement("div");
    const wrap = document.createElement("div");
    scroll.appendChild(wrap);
    document.body.appendChild(scroll);
    vi.spyOn(scroll, "getBoundingClientRect").mockReturnValue(rect(0, 0, 1000, 1000));
    vi.spyOn(wrap, "getBoundingClientRect").mockReturnValue(rect(0, 0, 4096, 4096));
    const tiles = createPdfTiles(() => {});
    tiles.refresh(page, 1, wrap, scroll, 3.5);
    expect(render).toHaveBeenCalledTimes(2);
    tiles.reset();
    tiles.refresh(page, 1, wrap, scroll, 4);
    expect(render).toHaveBeenCalledTimes(4);
    tiles.reset();
  });
});
