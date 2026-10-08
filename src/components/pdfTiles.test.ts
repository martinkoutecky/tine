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

  it("L13:86: overlapping pages settle within one viewer budget instead of rerendering each other", async () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const jobs: Array<() => void> = [];
    const render = vi.fn(() => {
      let finish!: () => void;
      const promise = new Promise<void>((resolve) => { finish = resolve; });
      jobs.push(finish);
      return { promise, cancel: vi.fn(finish) };
    });
    const page = { getViewport: () => ({ width: 16_000, height: 16_000 }), render } as unknown as pdfjs.PDFPageProxy;
    const scroll = document.createElement("div"); document.body.append(scroll);
    let viewportSize = 1000;
    vi.spyOn(scroll, "getBoundingClientRect").mockImplementation(() => rect(0, 0, viewportSize, viewportSize === 1000 ? 500 : viewportSize));
    const wraps = [1, 2].map(() => {
      const wrap = document.createElement("div"); scroll.append(wrap);
      vi.spyOn(wrap, "getBoundingClientRect").mockReturnValue(rect(0, 0, 16_000, 16_000));
      return wrap;
    });
    const tiles = createPdfTiles(() => { throw new Error("render failed"); });
    try {
      tiles.refresh(page, 1, wraps[0], scroll, 4);
      // Page A still has a pending task when page B starts filling the cap.
      jobs.shift()!(); for (let i = 0; i < 5; i++) await Promise.resolve();
      viewportSize = 10_000;
      tiles.refresh(page, 2, wraps[1], scroll, 4);
      for (let i = 0; i < 30; i++) {
        jobs.shift()?.();
        for (let j = 0; j < 5; j++) await Promise.resolve();
      }
      const settled = render.mock.calls.length;
      expect(settled, "one stable viewport must stop creating tile work").toBeLessThanOrEqual(10);
      expect(wraps.every((wrap) => wrap.querySelectorAll("canvas").length > 0)).toBe(true);
      tiles.releasePage(1);
      while (jobs.length) { jobs.shift()!(); for (let j = 0; j < 5; j++) await Promise.resolve(); }
      expect(wraps[0].querySelectorAll("canvas")).toHaveLength(0);
    } finally { tiles.reset(); }
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
