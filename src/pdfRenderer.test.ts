// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("pdfjs-dist/web/pdf_viewer.mjs", () => ({
  EventBus: class EventBus {
    private listeners = new Map<string, Set<(event: unknown) => void>>();
    on(name: string, listener: (event: unknown) => void) {
      const listeners = this.listeners.get(name) ?? new Set();
      listeners.add(listener);
      this.listeners.set(name, listeners);
    }
    off(name: string, listener: (event: unknown) => void) {
      this.listeners.get(name)?.delete(listener);
    }
    dispatch(name: string, event: unknown) {
      for (const listener of this.listeners.get(name) ?? []) listener(event);
    }
  },
  PDFPageView: class PDFPageView {},
}));

import { PdfRenderCoordinator } from "./pdfRenderCoordinator";
import {
  PdfPageViewRenderer,
  TINE_PDF_LOADING_OPTIONS,
  pdfPageViewScaleToTineScale,
  tineScaleToPdfPageViewScale,
  type DirectPdfPageView,
  type DirectPdfPageViewFactory,
  type DirectPdfPageViewOptions,
  type PdfPageProxyLike,
} from "./pdfRenderer";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

async function turns(count = 3): Promise<void> {
  for (let index = 0; index < count; index += 1) await Promise.resolve();
}

function fakePage(): PdfPageProxyLike & { cleanup: ReturnType<typeof vi.fn> } {
  return {
    cleanup: vi.fn(),
    getViewport: vi.fn(() => ({
      width: 612,
      height: 792,
      rotation: 0,
      clone: vi.fn(),
      convertToPdfPoint: vi.fn(),
    })),
  };
}

function fakeView(options: DirectPdfPageViewOptions): DirectPdfPageView & {
  destroy: ReturnType<typeof vi.fn>;
  cleanup: ReturnType<typeof vi.fn>;
} {
  const div = document.createElement("div");
  options.container.append(div);
  return {
    id: options.id,
    renderingId: `page${options.id}`,
    renderingState: 0,
    width: 612 * options.scale,
    height: 792 * options.scale,
    scale: options.scale,
    div,
    canvas: null,
    draw: vi.fn(async function (this: DirectPdfPageView) {
      this.renderingState = 3;
      (options.eventBus as unknown as { dispatch: (name: string, event: unknown) => void })
        .dispatch("pagerendered", { source: this });
    }),
    reset: vi.fn(),
    destroy: vi.fn(),
    cleanup: vi.fn(),
    setPdfPage: vi.fn(),
    update: vi.fn(),
  };
}

describe("direct PDFPageView renderer adapter", () => {
  beforeEach(() => document.body.replaceChildren());

  it("converts Tine display scale to compensate for PDF.js CSS units", () => {
    expect(tineScaleToPdfPageViewScale(2)).toBeCloseTo(1.5);
    expect(pdfPageViewScaleToTineScale(1.5)).toBeCloseTo(2);
    expect(() => tineScaleToPdfPageViewScale(0)).toThrow(/positive finite/);
  });

  it("constructs a queue-owned page view with text but no native annotations", async () => {
    const page = fakePage();
    const optionsSeen: DirectPdfPageViewOptions[] = [];
    const createPageView: DirectPdfPageViewFactory = (options) => {
      optionsSeen.push(options);
      return fakeView(options);
    };
    const renderer = new PdfPageViewRenderer({
      document: { getPage: vi.fn(async () => page) },
      coordinator: new PdfRenderCoordinator(10_000, 4_000),
      createPageView,
    });
    const host = document.createElement("div");
    renderer.setVisiblePages([1], true);

    const view = await renderer.mountPage(1, host, 2);
    await turns();

    expect(optionsSeen).toHaveLength(1);
    expect(optionsSeen[0]).toMatchObject({
      id: 1,
      scale: 1.5,
      textLayerMode: 1,
      annotationMode: 0,
      maxCanvasPixels: 4_000,
    });
    expect(optionsSeen[0].renderingQueue.hasViewer()).toBe(true);
    expect(view?.setPdfPage).toHaveBeenCalledWith(page);
    expect(view?.draw).toHaveBeenCalledOnce();
    renderer.dispose();
  });

  it("revalidates the page-slot generation after asynchronous getPage", async () => {
    const first = deferred<PdfPageProxyLike>();
    const second = deferred<PdfPageProxyLike>();
    const createPageView = vi.fn((options: DirectPdfPageViewOptions) => fakeView(options));
    const getPage = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const renderer = new PdfPageViewRenderer({
      document: { getPage },
      coordinator: new PdfRenderCoordinator(10_000, 4_000),
      createPageView,
    });
    const host = document.createElement("div");

    const obsoleteMount = renderer.mountPage(1, host, 1);
    const currentMount = renderer.mountPage(1, host, 1);
    const currentPage = fakePage();
    second.resolve(currentPage);
    const currentView = await currentMount;
    const obsoletePage = fakePage();
    first.resolve(obsoletePage);

    expect(await obsoleteMount).toBeNull();
    expect(currentView).not.toBeNull();
    expect(createPageView).toHaveBeenCalledOnce();
    expect(obsoletePage.cleanup).not.toHaveBeenCalled();
    renderer.dispose();
  });

  it("uses the latest scale when getPage completes after a zoom", async () => {
    const pending = deferred<PdfPageProxyLike>();
    const optionsSeen: DirectPdfPageViewOptions[] = [];
    const renderer = new PdfPageViewRenderer({
      document: { getPage: vi.fn(() => pending.promise) },
      coordinator: new PdfRenderCoordinator(10_000, 4_000),
      createPageView: (options) => {
        optionsSeen.push(options);
        return fakeView(options);
      },
    });
    const mount = renderer.mountPage(1, document.createElement("div"), 1);
    renderer.updateScale(1, 2);

    pending.resolve(fakePage());
    await mount;

    expect(optionsSeen[0].scale).toBeCloseTo(1.5);
    renderer.dispose();
  });

  it("derives a per-view pixel ceiling that also bounds an extreme dimension", async () => {
    const tallPage = fakePage();
    vi.mocked(tallPage.getViewport).mockReturnValue({
      width: 100,
      height: 40_000,
      rotation: 0,
      clone: vi.fn(),
      convertToPdfPoint: vi.fn(),
    });
    const renderer = new PdfPageViewRenderer({
      document: { getPage: vi.fn(async () => tallPage) },
      coordinator: new PdfRenderCoordinator(50_000_000, 20_000_000),
      createPageView: (options) => {
        const view = fakeView(options);
        view.width = 100;
        view.height = 40_000;
        return view;
      },
      maxCanvasDimension: 10_000,
    });

    const view = await renderer.mountPage(1, document.createElement("div"), 1);

    expect(view?.canvasPixelLimit).toBe(250_000);
    renderer.dispose();
  });

  it("invalidates late getPage completion when disposed", async () => {
    const pending = deferred<PdfPageProxyLike>();
    const createPageView = vi.fn((options: DirectPdfPageViewOptions) => fakeView(options));
    const renderer = new PdfPageViewRenderer({
      document: { getPage: vi.fn(() => pending.promise) },
      coordinator: new PdfRenderCoordinator(10_000, 4_000),
      createPageView,
    });
    const mount = renderer.mountPage(1, document.createElement("div"), 1);

    renderer.dispose();
    const latePage = fakePage();
    pending.resolve(latePage);

    expect(await mount).toBeNull();
    expect(createPageView).not.toHaveBeenCalled();
    expect(latePage.cleanup).not.toHaveBeenCalled();
  });

  it("disposes view resources with reset and never destroys shared proxies", async () => {
    const page = fakePage();
    const viewRef: { current?: ReturnType<typeof fakeView> } = {};
    const renderer = new PdfPageViewRenderer({
      document: { getPage: vi.fn(async () => page) },
      coordinator: new PdfRenderCoordinator(10_000, 4_000),
      createPageView: (options) => (viewRef.current = fakeView(options)),
    });
    const host = document.createElement("div");
    await renderer.mountPage(1, host, 1);

    renderer.dispose();

    expect(viewRef.current?.reset).toHaveBeenCalledOnce();
    expect(viewRef.current?.destroy).not.toHaveBeenCalled();
    expect(viewRef.current?.cleanup).not.toHaveBeenCalled();
    expect(page.cleanup).not.toHaveBeenCalled();
    expect(host.childElementCount).toBe(0);
  });

  it("publishes the lazy document-loading flag used by the later session owner", () => {
    expect(TINE_PDF_LOADING_OPTIONS).toEqual({ disableAutoFetch: true });
  });
});
