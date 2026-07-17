import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { Show, createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import {
  KeyedPdfViewer,
  PdfViewer,
  PDF_CANVAS_CACHE_PIXEL_BUDGET,
  PDF_FIND_MATCH_CAP,
  isPdfAreaModifier,
} from "./PdfViewer";
import {
  clearTransientLayersForTest,
  dismissTopTransient,
  registerTransientLayer,
} from "../transientLayers";

const getDocumentMock = vi.hoisted(() => vi.fn());

vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: {},
  getDocument: getDocumentMock,
  TextLayer: class {
    render() {
      return Promise.resolve();
    }

    update() {
      return Promise.resolve();
    }
  },
}));

vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({
  default: "pdf.worker.test.js",
}));

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
}

class TestIntersectionObserver {
  static instances: TestIntersectionObserver[] = [];
  readonly elements: Element[] = [];

  constructor(private readonly callback: IntersectionObserverCallback) {
    TestIntersectionObserver.instances.push(this);
  }

  observe(element: Element) {
    this.elements.push(element);
  }

  unobserve() {}
  disconnect() {}
  takeRecords() { return []; }

  show(element: Element) {
    this.callback([{ isIntersecting: true, target: element } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }

  hide(element: Element) {
    this.callback([{ isIntersecting: false, target: element } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
}

function page(width: number, height: number) {
  return {
    getViewport: vi.fn(({ scale }: { scale: number }) => ({ width: width * scale, height: height * scale })),
    getTextContent: vi.fn().mockResolvedValue({ items: [] }),
    render: vi.fn().mockReturnValue({ promise: Promise.resolve(), cancel: vi.fn() }),
  };
}

function documentWithPages(pages: ReturnType<typeof page>[]) {
  return {
    numPages: pages.length,
    getPage: vi.fn((number: number) => Promise.resolve(pages[number - 1])),
    getOutline: vi.fn().mockResolvedValue([]),
    getDestination: vi.fn().mockResolvedValue(null),
    getPageIndex: vi.fn().mockResolvedValue(0),
    destroy: vi.fn().mockResolvedValue(undefined),
  };
}

describe("PdfViewer resource safety", () => {
  beforeEach(() => {
    getDocumentMock.mockReset();
    TestIntersectionObserver.instances = [];
    vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(),
    });
    vi.spyOn(backend(), "readHighlights").mockResolvedValue([]);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.replaceChildren();
    Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
    Reflect.deleteProperty(document, "elementFromPoint");
  });

  it("shows an error and creates no page wrappers when pdf.js rejects the document", async () => {
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1, 2, 3]));
    getDocumentMock.mockReturnValue({ promise: Promise.reject(new Error("invalid pdf")) });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="bad.pdf" label="Bad PDF" />, host);
    try {
      await flush();

      expect(host.querySelector(".pdf-load-error")?.textContent).toContain("Couldn't load this PDF");
      expect(host.querySelector(".pdf-load-error")?.textContent).toContain("invalid pdf");
      expect(host.querySelector(".pdf-page")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("rejects an oversized PDF before handing its bytes to pdf.js", async () => {
    const byteLength = 256 * 1024 * 1024 + 1;
    vi.spyOn(backend(), "readAsset").mockResolvedValue({ length: byteLength, byteLength } as Uint8Array);

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="huge.pdf" label="Huge PDF" />, host);
    try {
      await flush();

      expect(host.querySelector(".pdf-load-error")?.textContent).toContain("larger than 256 MiB");
      expect(getDocumentMock).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("rejects a page count that would create too many layout nodes", async () => {
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const pdf = {
      numPages: 5001,
      getPage: vi.fn(),
      destroy: vi.fn().mockResolvedValue(undefined),
    };
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(pdf) });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="many.pdf" label="Many pages" />, host);
    try {
      await flush();

      expect(host.querySelector(".pdf-load-error")?.textContent).toContain("at most 5000 pages");
      expect(host.querySelector(".pdf-page")).toBeNull();
      expect(pdf.getPage).not.toHaveBeenCalled();
      expect(pdf.destroy).toHaveBeenCalledOnce();
    } finally {
      dispose();
    }
  });

  it("rejects unsafe dimensions on the first page before building the layout", async () => {
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const pdf = documentWithPages([page(14_401, 792)]);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(pdf) });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="wide.pdf" label="Wide PDF" />, host);
    try {
      await flush();

      expect(host.querySelector(".pdf-load-error")?.textContent).toContain("page 1 is too large");
      expect(host.querySelector(".pdf-page")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("rejects unsafe dimensions discovered on a later page", async () => {
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const pdf = documentWithPages([page(612, 792), page(20_000, 100)]);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(pdf) });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="mixed.pdf" label="Mixed PDF" />, host);
    try {
      await flush();
      const secondPage = host.querySelectorAll(".pdf-page")[1];
      TestIntersectionObserver.instances[0].show(secondPage);
      await flush();

      expect(host.querySelector(".pdf-load-error")?.textContent).toContain("page 2 is too large");
      expect(host.querySelector(".pdf-page")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("downsamples a valid large page to a bounded canvas allocation", async () => {
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(1400);
    vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(2);
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const largePage = page(1000, 14_000);
    const pdf = documentWithPages([largePage]);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(pdf) });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="poster.pdf" label="Poster PDF" />, host);
    try {
      await flush();
      const pageElement = host.querySelector(".pdf-page")!;
      TestIntersectionObserver.instances[0].show(pageElement);
      await flush();

      const canvas = pageElement.querySelector("canvas")!;
      expect(host.querySelector(".pdf-load-error")).toBeNull();
      expect(canvas.width).toBeLessThanOrEqual(16_384);
      expect(canvas.height).toBeLessThanOrEqual(16_384);
      expect(canvas.width * canvas.height).toBeLessThanOrEqual(16_777_216);
      expect(largePage.render).toHaveBeenCalledOnce();
      expect(largePage.render.mock.calls[0][0].transform[0]).toBeLessThan(2);
    } finally {
      dispose();
    }
  });

  it("bounds all retained backing stores by pixels and zeroes each evicted canvas", async () => {
    vi.spyOn(backend() as any, "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 4 });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    vi.spyOn(window, "devicePixelRatio", "get").mockReturnValue(2);
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const pdf = documentWithPages(Array.from({ length: 5 }, () => page(612, 792)));
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(pdf) });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="long.pdf" label="Long PDF" />, host);
    try {
      await flush();
      const pageElements = [...host.querySelectorAll<HTMLElement>(".pdf-page")];
      const observer = TestIntersectionObserver.instances[0];
      let firstCanvas: HTMLCanvasElement | null = null;
      for (const element of pageElements) {
        observer.show(element);
        await flush();
        firstCanvas ??= element.querySelector("canvas");
        observer.hide(element);
      }

      const retained = [...host.querySelectorAll<HTMLCanvasElement>(".pdf-page canvas")];
      const retainedPixels = retained.reduce((total, canvas) => total + canvas.width * canvas.height, 0);
      expect(retainedPixels).toBeLessThanOrEqual(PDF_CANVAS_CACHE_PIXEL_BUDGET);
      expect(retained.length).toBeLessThan(pageElements.length);
      expect(firstCanvas?.isConnected).toBe(false);
      expect(firstCanvas?.width).toBe(0);
      expect(firstCanvas?.height).toBe(0);
    } finally {
      dispose();
    }
  });
});

describe("PdfViewer OG area-highlight selection", () => {
  beforeEach(() => {
    clearTransientLayersForTest();
    getDocumentMock.mockReset();
    TestIntersectionObserver.instances = [];
    vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(),
    });
    vi.spyOn(backend() as any, "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
      drawImage: vi.fn(),
    } as unknown as CanvasRenderingContext2D);
    vi.spyOn(HTMLCanvasElement.prototype, "toBlob").mockImplementation((callback) => {
      callback({
        arrayBuffer: async () => new Uint8Array([1, 2, 3]).buffer,
      } as Blob);
    });
    getDocumentMock.mockReturnValue({
      promise: Promise.resolve(documentWithPages([page(612, 792)])),
    });
  });

  afterEach(() => {
    clearTransientLayersForTest();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.replaceChildren();
    Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
    Reflect.deleteProperty(document, "elementFromPoint");
  });

  async function mountAreaViewer() {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="paper.pdf" label="Paper" />, host);
    await flush();
    const wrap = host.querySelector(".pdf-page") as HTMLDivElement;
    vi.spyOn(wrap, "getBoundingClientRect").mockReturnValue({
      left: 0, top: 0, right: 612, bottom: 792, width: 612, height: 792, x: 0, y: 0,
      toJSON: () => ({}),
    });
    return { host, wrap, dispose };
  }

  function dragArea(
    wrap: HTMLElement,
    end: { x: number; y: number },
    modifiers: { ctrlKey?: boolean; metaKey?: boolean; shiftKey?: boolean } = {}
  ) {
    wrap.dispatchEvent(new MouseEvent("mousedown", {
      bubbles: true,
      cancelable: true,
      button: 0,
      clientX: 20,
      clientY: 30,
      ...modifiers,
    }));
    window.dispatchEvent(new MouseEvent("mousemove", {
      bubbles: true,
      clientX: end.x,
      clientY: end.y,
      ...modifiers,
    }));
    window.dispatchEvent(new MouseEvent("mouseup", {
      bubbles: true,
      clientX: end.x,
      clientY: end.y,
      ...modifiers,
    }));
  }

  it("maps direct area selection to Command on macOS and Shift elsewhere", () => {
    expect(isPdfAreaModifier({ metaKey: true, shiftKey: false }, true)).toBe(true);
    expect(isPdfAreaModifier({ metaKey: false, shiftKey: true }, true)).toBe(false);
    expect(isPdfAreaModifier({ metaKey: false, shiftKey: true }, false)).toBe(true);
    expect(isPdfAreaModifier({ metaKey: true, shiftKey: false }, false)).toBe(false);
  });

  it("opens the area color chooser from a non-macOS Shift drag", async () => {
    const saveArea = vi.spyOn(backend(), "savePdfAreaImage").mockResolvedValue("");
    const writeHighlights = vi.spyOn(backend(), "writeHighlights").mockResolvedValue(undefined);
    const { host, wrap, dispose } = await mountAreaViewer();
    try {
      dragArea(wrap, { x: 40, y: 50 }, { shiftKey: true });
      await flush();

      expect(host.querySelectorAll(".pdf-color-swatch")).toHaveLength(5);
      expect(saveArea).not.toHaveBeenCalled();
      expect(writeHighlights).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("does not start direct area selection from Control alone off macOS", async () => {
    const saveArea = vi.spyOn(backend(), "savePdfAreaImage").mockResolvedValue("");
    const writeHighlights = vi.spyOn(backend(), "writeHighlights").mockResolvedValue(undefined);
    const { host, wrap, dispose } = await mountAreaViewer();
    try {
      dragArea(wrap, { x: 40, y: 50 }, { ctrlKey: true });
      await flush();

      expect(host.querySelector(".pdf-color-menu")).toBeNull();
      expect(saveArea).not.toHaveBeenCalled();
      expect(writeHighlights).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("requires both toolbar-area dimensions to be strictly greater than 10 CSS pixels", async () => {
    vi.mocked(backend().openPdf).mockResolvedValue({ highlights: [], page: 1, scale: 2 });
    const saveArea = vi.spyOn(backend(), "savePdfAreaImage").mockResolvedValue("");
    const { host, wrap, dispose } = await mountAreaViewer();
    try {
      (host.querySelector('button[title^="Area highlight"]') as HTMLButtonElement).click();
      dragArea(wrap, { x: 30, y: 50 });
      await flush();
      expect(host.querySelector(".pdf-color-menu")).toBeNull();
      expect(saveArea).not.toHaveBeenCalled();

      dragArea(wrap, { x: 31, y: 41 });
      await flush();
      expect(host.querySelectorAll(".pdf-color-swatch")).toHaveLength(5);
      expect(saveArea).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("defers toolbar area writes until color choice and dismisses without changing selection or view", async () => {
    const id = "33333333-3333-4333-8333-333333333333";
    vi.spyOn(crypto, "randomUUID").mockReturnValue(id);
    vi.spyOn(Date, "now").mockReturnValue(1234);
    const saveArea = vi.spyOn(backend(), "savePdfAreaImage").mockResolvedValue("");
    const writeHighlights = vi.spyOn(backend(), "writeHighlights").mockResolvedValue(undefined);
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    const writeViewState = vi.spyOn(backend(), "writePdfViewState").mockResolvedValue(undefined);
    const selection = { removeAllRanges: vi.fn() } as unknown as Selection;
    vi.spyOn(window, "getSelection").mockReturnValue(selection);
    const { host, wrap, dispose } = await mountAreaViewer();
    try {
      (host.querySelector('button[title^="Area highlight"]') as HTMLButtonElement).click();
      dragArea(wrap, { x: 45, y: 55 });
      await flush();

      expect(host.querySelectorAll(".pdf-color-swatch")).toHaveLength(5);
      expect(saveArea).not.toHaveBeenCalled();
      expect(writeHighlights).not.toHaveBeenCalled();
      expect(writeText).not.toHaveBeenCalled();
      document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
      await flush();
      expect(host.querySelector(".pdf-color-menu")).toBeNull();
      expect(saveArea).not.toHaveBeenCalled();
      expect(writeHighlights).not.toHaveBeenCalled();
      expect(writeText).not.toHaveBeenCalled();

      dragArea(wrap, { x: 45, y: 55 });
      await flush();
      expect(host.querySelectorAll(".pdf-color-swatch")).toHaveLength(5);
      expect(dismissTopTransient("escape")).toBe(true);
      await flush();
      expect(host.querySelector(".pdf-color-menu")).toBeNull();
      expect(saveArea).not.toHaveBeenCalled();
      expect(writeHighlights).not.toHaveBeenCalled();
      expect(writeText).not.toHaveBeenCalled();
      expect(selection.removeAllRanges).not.toHaveBeenCalled();
      expect(writeViewState).not.toHaveBeenCalled();
      expect((host.querySelector(".pdf-page-input") as HTMLInputElement).value).toBe("1");
      expect(host.querySelector(".pdf-zoom-level")?.textContent).toBe("100%");

      dragArea(wrap, { x: 45, y: 55 });
      await flush();
      const blue = host.querySelectorAll<HTMLButtonElement>(".pdf-color-swatch")[2];
      blue.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      await flush();
      await flush();

      expect(saveArea).toHaveBeenCalledWith("paper.pdf", 1, id, 1234, new Uint8Array([1, 2, 3]));
      expect(writeHighlights).toHaveBeenCalledOnce();
      expect(writeHighlights.mock.calls[0][2]).toEqual([
        expect.objectContaining({ id, page: 1, color: "blue", text: null, image: 1234 }),
      ]);
      expect(writeHighlights.mock.calls[0][3]).toEqual([]);
      expect(writeText).toHaveBeenCalledWith(`((${id}))`);
      expect(selection.removeAllRanges).not.toHaveBeenCalled();
      expect(writeViewState).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });
});

describe("PdfViewer OG state and reference behavior", () => {
  beforeEach(() => {
    getDocumentMock.mockReset();
    TestIntersectionObserver.instances = [];
    vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(),
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.replaceChildren();
    Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
    Reflect.deleteProperty(document, "elementFromPoint");
  });

  it("restores OG page and scale then debounces changed view state", async () => {
    const openPdf = vi.spyOn(backend() as any, "openPdf").mockResolvedValue({
      highlights: [],
      page: 2,
      scale: 2,
    });
    const writeState = vi.spyOn(backend() as any, "writePdfViewState").mockResolvedValue(undefined);
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const pdf = documentWithPages([page(612, 792), page(612, 792)]);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(pdf) });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="paper.pdf" label="Paper" />, host);
    try {
      await flush();
      expect(openPdf).toHaveBeenCalledWith("paper.pdf", "Paper");
      expect((host.querySelector(".pdf-page-input") as HTMLInputElement).value).toBe("2");
      expect(host.querySelector(".pdf-zoom-level")?.textContent).toBe("200%");
      expect(writeState).not.toHaveBeenCalled();

      vi.useFakeTimers();
      (host.querySelector('button[title="Zoom in"]') as HTMLButtonElement).click();
      await vi.advanceTimersByTimeAsync(3999);
      expect(writeState).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(writeState).toHaveBeenCalledWith("paper.pdf", 2, 2.2);
    } finally {
      dispose();
    }
  });

  it("copies a newly persisted highlight block reference like OG", async () => {
    vi.spyOn(backend() as any, "openPdf").mockResolvedValue({ highlights: [], page: null, scale: null });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const writeHighlights = vi.spyOn(backend(), "writeHighlights").mockResolvedValue(undefined);
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(documentWithPages([page(612, 792)])) });
    vi.spyOn(crypto, "randomUUID").mockReturnValue("11111111-1111-4111-8111-111111111111");

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="paper.pdf" label="Paper" />, host);
    try {
      await flush();
      const wrap = host.querySelector(".pdf-page") as HTMLDivElement;
      vi.spyOn(wrap, "getBoundingClientRect").mockReturnValue({
        left: 0, top: 0, right: 612, bottom: 792, width: 612, height: 792, x: 0, y: 0,
        toJSON: () => ({}),
      });
      vi.mocked(document.elementFromPoint).mockReturnValue(wrap);
      const selection = {
        isCollapsed: false,
        toString: () => "selected text",
        getRangeAt: () => ({
          getClientRects: () => [{ left: 10, top: 20, right: 110, bottom: 32, width: 100, height: 12 }],
        }),
        removeAllRanges: vi.fn(),
      } as unknown as Selection;
      vi.spyOn(window, "getSelection").mockReturnValue(selection);

      host.querySelector(".pdf-scroll")!.dispatchEvent(new MouseEvent("mouseup", {
        bubbles: true,
        clientX: 20,
        clientY: 30,
      }));
      await flush();
      (host.querySelector(".pdf-color-swatch") as HTMLButtonElement).dispatchEvent(
        new MouseEvent("mousedown", { bubbles: true })
      );
      await flush();

      expect(writeHighlights).toHaveBeenCalledOnce();
      expect(writeText).toHaveBeenCalledWith("((11111111-1111-4111-8111-111111111111))");
    } finally {
      dispose();
    }
  });

  it("offers OG reference actions for existing text and area highlights", async () => {
    const textId = "11111111-1111-4111-8111-111111111111";
    const areaId = "22222222-2222-4222-8222-222222222222";
    const rect = { top: 40, left: 20, width: 80, height: 12 };
    vi.spyOn(backend() as any, "openPdf").mockResolvedValue({
      highlights: [
        {
          id: textId,
          page: 1,
          position: { page: 1, bounding: rect, rects: [rect] },
          color: "yellow",
          text: "existing text highlight",
          image: null,
        },
        {
          id: areaId,
          page: 1,
          position: { page: 1, bounding: { ...rect, top: 80 }, rects: [] },
          color: "green",
          text: null,
          image: 1234,
        },
      ],
      page: 1,
      scale: 1,
    });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const writeHighlights = vi.spyOn(backend(), "writeHighlights").mockResolvedValue(undefined);
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(documentWithPages([page(612, 792)])) });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="paper.pdf" label="Paper" />, host);
    try {
      await flush();
      TestIntersectionObserver.instances[0].show(host.querySelector(".pdf-page")!);
      await flush();
      const textHighlight = host.querySelector(`[data-highlight-id="${textId}"]`) as HTMLElement;
      expect(textHighlight).not.toBeNull();
      textHighlight.click();
      await flush();

      const actionLabels = [...host.querySelectorAll<HTMLButtonElement>(".pdf-color-menu button")]
        .map((button) => button.textContent?.trim())
        .filter(Boolean);
      expect(actionLabels).toEqual(expect.arrayContaining(["Copy ref", "Linked references"]));

      const copy = [...host.querySelectorAll<HTMLButtonElement>(".pdf-color-menu button")]
        .find((button) => button.textContent?.trim() === "Copy ref")!;
      copy.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      await flush();
      expect(writeHighlights).toHaveBeenCalledOnce();
      expect(writeHighlights.mock.calls[0][2].map((highlight) => highlight.id)).toEqual([textId, areaId]);
      expect(writeHighlights.mock.calls[0][3]).toEqual([textId, areaId]);
      expect(writeText).toHaveBeenCalledWith(`((${textId}))`);

      const areaHighlight = host.querySelector(`[data-highlight-id="${areaId}"]`) as HTMLElement;
      const contextMenu = new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 30,
        clientY: 90,
      });
      areaHighlight.dispatchEvent(contextMenu);
      await flush();
      expect(contextMenu.defaultPrevented).toBe(true);
      const areaActionLabels = [...host.querySelectorAll<HTMLButtonElement>(".pdf-color-menu button")]
        .map((button) => button.textContent?.trim())
        .filter(Boolean);
      expect(areaActionLabels).toEqual(expect.arrayContaining(["Copy ref", "Linked references"]));
    } finally {
      dispose();
    }
  });

  it("tears down the complete document identity before opening another PDF", async () => {
    const openPdf = vi.spyOn(backend() as any, "openPdf").mockImplementation(async (...args: unknown[]) => {
      const filename = String(args[0]);
      return {
        highlights: [],
        page: filename === "a.pdf" ? 2 : 1,
        scale: filename === "a.pdf" ? 2 : 1,
      };
    });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const first = documentWithPages([page(612, 792), page(612, 792)]);
    const second = documentWithPages([page(612, 792)]);
    getDocumentMock
      .mockReturnValueOnce({ promise: Promise.resolve(first) })
      .mockReturnValueOnce({ promise: Promise.resolve(second) });
    const [target, setTarget] = createSignal({ filename: "a.pdf", label: "A" });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <KeyedPdfViewer target={target} />, host);
    try {
      await flush();
      expect(host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename")).toBe("a.pdf");

      setTarget({ filename: "b.pdf", label: "B" });
      await flush();

      expect(openPdf.mock.calls.map(([filename]) => filename)).toEqual(["a.pdf", "b.pdf"]);
      expect(first.destroy).toHaveBeenCalledOnce();
      expect(host.querySelectorAll(".pdf-viewer")).toHaveLength(1);
      expect(host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename")).toBe("b.pdf");
      expect(host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready")).toBe("true");
      expect((host.querySelector(".pdf-page-input") as HTMLInputElement).value).toBe("1");
      expect(host.querySelector(".pdf-zoom-level")?.textContent).toBe("100%");
    } finally {
      dispose();
    }
  });

  it("destroys a late document load after its asset identity was replaced", async () => {
    vi.spyOn(backend() as any, "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const stale = documentWithPages([page(612, 792)]);
    const current = documentWithPages([page(612, 792)]);
    let resolveStale!: (document: typeof stale) => void;
    const staleLoad = new Promise<typeof stale>((resolve) => { resolveStale = resolve; });
    getDocumentMock
      .mockReturnValueOnce({ promise: staleLoad })
      .mockReturnValueOnce({ promise: Promise.resolve(current) });
    const [target, setTarget] = createSignal({ filename: "a.pdf", label: "A" });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <KeyedPdfViewer target={target} />, host);
    try {
      await flush();
      setTarget({ filename: "b.pdf", label: "B" });
      await flush();
      resolveStale(stale);
      await flush();

      expect(stale.destroy).toHaveBeenCalledOnce();
      expect(current.destroy).not.toHaveBeenCalled();
      expect(host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename")).toBe("b.pdf");
      expect(host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready")).toBe("true");
    } finally {
      dispose();
    }
  });

  it("keeps one PDF mounted and scrolls repeated references to the exact highlight", async () => {
    const firstId = "11111111-1111-4111-8111-111111111111";
    const secondId = "22222222-2222-4222-8222-222222222222";
    const rect = (top: number) => ({ top, left: 20, width: 80, height: 12 });
    const openPdf = vi.spyOn(backend() as any, "openPdf").mockResolvedValue({
      highlights: [
        {
          id: firstId,
          page: 1,
          position: { page: 1, bounding: rect(40), rects: [rect(40)] },
          color: "yellow",
          text: "first",
          image: null,
        },
        {
          id: secondId,
          page: 1,
          position: { page: 1, bounding: rect(500), rects: [rect(500)] },
          color: "green",
          text: "second",
          image: null,
        },
      ],
      page: 1,
      scale: 1,
    });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const pdf = documentWithPages([page(612, 792)]);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(pdf) });
    const [target, setTarget] = createSignal({
      filename: "paper.pdf",
      label: "Paper",
      page: 1,
      highlightId: firstId,
    });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <KeyedPdfViewer target={target} />, host);
    try {
      await flush();
      const viewer = host.querySelector(".pdf-viewer");
      expect(viewer?.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id")).toBe(firstId);

      setTarget({ filename: "paper.pdf", label: "Paper", page: 1, highlightId: secondId });
      await flush();

      expect(openPdf).toHaveBeenCalledOnce();
      expect(host.querySelector(".pdf-viewer")).toBe(viewer);
      expect(pdf.destroy).not.toHaveBeenCalled();
      expect(viewer?.getAttribute("data-pdf-highlight-target")).toBe(secondId);
      expect(viewer?.querySelector(".pdf-hl-target")?.getAttribute("data-highlight-id")).toBe(secondId);
    } finally {
      dispose();
    }
  });

  it("caps PDF Find occurrences and labels the result as truncated", async () => {
    vi.useFakeTimers();
    vi.spyOn(backend() as any, "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    const textPage = page(612, 792);
    textPage.getTextContent.mockResolvedValue({ items: [{ str: "a".repeat(PDF_FIND_MATCH_CAP + 500) }] });
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(documentWithPages([textPage])) });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="search.pdf" label="Search" />, host);
    try {
      await flush();
      (host.querySelector('button[title="Find in document (Ctrl+F)"]') as HTMLButtonElement).click();
      const input = host.querySelector(".pdf-find-input") as HTMLInputElement;
      input.value = "a";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await vi.advanceTimersByTimeAsync(180);
      await flush();

      expect(host.querySelector(".pdf-find-count")?.textContent).toBe(`1 / ${PDF_FIND_MATCH_CAP}+`);
    } finally {
      dispose();
    }
  });
});

describe("PdfViewer local transient ownership", () => {
  beforeEach(() => {
    clearTransientLayersForTest();
    getDocumentMock.mockReset();
    TestIntersectionObserver.instances = [];
    vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(),
    });
    vi.spyOn(backend() as any, "openPdf").mockResolvedValue({
      highlights: [],
      page: 2,
      scale: 2,
    });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    getDocumentMock.mockImplementation(() => ({
      promise: Promise.resolve(documentWithPages([page(612, 792), page(612, 792)])),
    }));
  });

  afterEach(() => {
    clearTransientLayersForTest();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.replaceChildren();
    Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
    Reflect.deleteProperty(document, "elementFromPoint");
  });

  it("owns Find above a lower transient for Escape and Back without losing viewer state or query", async () => {
    const writeViewState = vi.spyOn(backend() as any, "writePdfViewState").mockResolvedValue(undefined);
    const selection = { removeAllRanges: vi.fn() } as unknown as Selection;
    vi.spyOn(window, "getSelection").mockReturnValue(selection);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="search.pdf" label="Search" />, host);
    let lowerDismissals = 0;
    const unregisterLower = registerTransientLayer({
      id: "pdf-find-lower",
      dismiss: () => { lowerDismissals += 1; return true; },
    });
    try {
      await flush();
      const findButton = host.querySelector('button[title="Find in document (Ctrl+F)"]') as HTMLButtonElement;
      findButton.click();
      const input = host.querySelector(".pdf-find-input") as HTMLInputElement;
      input.value = "retained query";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));

      expect(dismissTopTransient("escape")).toBe(true);
      await flush();
      expect(host.querySelector(".pdf-find-bar")).toBeNull();
      expect(lowerDismissals).toBe(0);
      expect((host.querySelector(".pdf-page-input") as HTMLInputElement).value).toBe("2");
      expect(host.querySelector(".pdf-zoom-level")?.textContent).toBe("200%");
      expect(writeViewState).not.toHaveBeenCalled();

      findButton.click();
      expect((host.querySelector(".pdf-find-input") as HTMLInputElement).value).toBe("retained query");
      expect(dismissTopTransient("back")).toBe(true);
      await flush();
      expect(host.querySelector(".pdf-find-bar")).toBeNull();
      expect(lowerDismissals).toBe(0);

      expect(dismissTopTransient("escape")).toBe(true);
      expect(lowerDismissals).toBe(1);
    } finally {
      unregisterLower();
      dispose();
    }
  });

  it("dismisses the highlight menu only, preserving highlight, selection, and view state", async () => {
    const highlightId = "11111111-1111-4111-8111-111111111111";
    const rect = { top: 40, left: 20, width: 80, height: 12 };
    vi.mocked(backend().openPdf).mockResolvedValue({
      highlights: [{
        id: highlightId,
        page: 1,
        position: { page: 1, bounding: rect, rects: [rect] },
        color: "yellow",
        text: "existing text highlight",
        image: null,
      }],
      page: 2,
      scale: 2,
    });
    const writeHighlights = vi.spyOn(backend(), "writeHighlights").mockResolvedValue(undefined);
    const writeViewState = vi.spyOn(backend() as any, "writePdfViewState").mockResolvedValue(undefined);
    const selection = { removeAllRanges: vi.fn() } as unknown as Selection;
    vi.spyOn(window, "getSelection").mockReturnValue(selection);
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="marked.pdf" label="Marked" />, host);
    let lowerDismissals = 0;
    const unregisterLower = registerTransientLayer({
      id: "pdf-menu-lower",
      dismiss: () => { lowerDismissals += 1; return true; },
    });
    try {
      await flush();
      TestIntersectionObserver.instances[0].show(host.querySelector(".pdf-page")!);
      await flush();
      const highlight = host.querySelector(`[data-highlight-id="${highlightId}"]`) as HTMLElement;
      highlight.click();
      await flush();
      expect(host.querySelector(".pdf-color-menu")).not.toBeNull();

      expect(dismissTopTransient("escape")).toBe(true);
      await flush();
      expect(host.querySelector(".pdf-color-menu")).toBeNull();
      expect(host.querySelector(`[data-highlight-id="${highlightId}"]`)).not.toBeNull();
      expect(lowerDismissals).toBe(0);
      expect(writeHighlights).not.toHaveBeenCalled();
      expect(writeViewState).not.toHaveBeenCalled();
      expect(selection.removeAllRanges).not.toHaveBeenCalled();
      expect((host.querySelector(".pdf-page-input") as HTMLInputElement).value).toBe("2");
      expect(host.querySelector(".pdf-zoom-level")?.textContent).toBe("200%");

      highlight.click();
      await flush();
      expect(dismissTopTransient("back")).toBe(true);
      await flush();
      expect(host.querySelector(".pdf-color-menu")).toBeNull();
      expect(lowerDismissals).toBe(0);
    } finally {
      unregisterLower();
      dispose();
    }
  });

  it("orders simultaneous Find and highlight-menu peers by their latest interaction", async () => {
    const selection = {
      isCollapsed: false,
      toString: () => "selected text",
      getRangeAt: () => ({
        getClientRects: () => [{ left: 10, top: 20, right: 110, bottom: 32, width: 100, height: 12 }],
      }),
      removeAllRanges: vi.fn(),
    } as unknown as Selection;
    vi.spyOn(window, "getSelection").mockReturnValue(selection);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename="peers.pdf" label="Peers" />, host);
    let lowerDismissals = 0;
    const unregisterLower = registerTransientLayer({
      id: "pdf-peer-lower",
      dismiss: () => { lowerDismissals += 1; return true; },
    });
    try {
      await flush();
      const findButton = host.querySelector('button[title="Find in document (Ctrl+F)"]') as HTMLButtonElement;
      findButton.click();
      const pageElement = host.querySelector(".pdf-page") as HTMLElement;
      vi.spyOn(pageElement, "getBoundingClientRect").mockReturnValue({
        left: 0, top: 0, right: 612, bottom: 792, width: 612, height: 792, x: 0, y: 0,
        toJSON: () => ({}),
      });
      pageElement.dispatchEvent(new MouseEvent("mouseup", {
        bubbles: true,
        clientX: 20,
        clientY: 30,
      }));
      await flush();
      expect(host.querySelector(".pdf-find-bar")).not.toBeNull();
      expect(host.querySelector(".pdf-color-menu")).not.toBeNull();

      const findInput = host.querySelector(".pdf-find-input") as HTMLInputElement;
      findInput.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
      expect(dismissTopTransient("escape")).toBe(true);
      await flush();
      expect(host.querySelector(".pdf-find-bar")).toBeNull();
      expect(host.querySelector(".pdf-color-menu")).not.toBeNull();
      expect(lowerDismissals).toBe(0);

      findButton.click();
      const menuRoot = host.querySelector(".pdf-color-menu") as HTMLElement;
      menuRoot.dispatchEvent(new Event("pointerdown", { bubbles: true }));
      expect(dismissTopTransient("back")).toBe(true);
      await flush();
      expect(host.querySelector(".pdf-color-menu")).toBeNull();
      expect(host.querySelector(".pdf-find-bar")).not.toBeNull();
      expect(lowerDismissals).toBe(0);
    } finally {
      unregisterLower();
      dispose();
    }
  });

  it("keeps two same-filename viewers independently registered and removes owners on explicit close or unmount", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const [showSecond, setShowSecond] = createSignal(true);
    const dispose = render(() => (
      <>
        <PdfViewer filename="same.pdf" label="First" />
        <Show when={showSecond()}>
          <PdfViewer filename="same.pdf" label="Second" />
        </Show>
      </>
    ), host);
    let lowerDismissals = 0;
    const unregisterLower = registerTransientLayer({
      id: "pdf-instance-lower",
      dismiss: () => { lowerDismissals += 1; return true; },
    });
    try {
      await flush();
      const viewers = [...host.querySelectorAll<HTMLElement>(".pdf-viewer")];
      for (const viewer of viewers) {
        (viewer.querySelector('button[title="Find in document (Ctrl+F)"]') as HTMLButtonElement).click();
      }
      expect(viewers.every((viewer) => viewer.querySelector(".pdf-find-bar"))).toBe(true);

      expect(dismissTopTransient("back")).toBe(true);
      await flush();
      expect(viewers[0].querySelector(".pdf-find-bar")).not.toBeNull();
      expect(viewers[1].querySelector(".pdf-find-bar")).toBeNull();
      expect(lowerDismissals).toBe(0);

      (viewers[1].querySelector('button[title="Find in document (Ctrl+F)"]') as HTMLButtonElement).click();
      await flush();
      expect(viewers[1].querySelector(".pdf-find-bar")).not.toBeNull();
      setShowSecond(false);
      await flush();
      expect(dismissTopTransient("escape")).toBe(true);
      await flush();
      expect(viewers[0].querySelector(".pdf-find-bar")).toBeNull();
      expect(lowerDismissals).toBe(0);

      const firstFindButton = viewers[0].querySelector('button[title="Find in document (Ctrl+F)"]') as HTMLButtonElement;
      firstFindButton.click();
      await flush();
      expect(viewers[0].querySelector(".pdf-find-bar")).not.toBeNull();
      firstFindButton.click();
      await flush();
      expect(viewers[0].querySelector(".pdf-find-bar")).toBeNull();
      expect(dismissTopTransient("escape")).toBe(true);
      expect(lowerDismissals).toBe(1);
    } finally {
      unregisterLower();
      dispose();
    }
  });
});

describe("PdfViewer released-OG themes and outline", () => {
  beforeEach(() => {
    clearTransientLayersForTest();
    getDocumentMock.mockReset();
    TestIntersectionObserver.instances = [];
    vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(),
    });
    localStorage.clear();
    vi.spyOn(backend() as any, "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
  });

  afterEach(() => {
    clearTransientLayersForTest();
    localStorage.clear();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.replaceChildren();
    Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
    Reflect.deleteProperty(document, "elementFromPoint");
  });

  function mountViewer(filename = "paper.pdf") {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer filename={filename} label="Paper" />, host);
    return { host, dispose };
  }

  function button(host: HTMLElement, selector: string): HTMLButtonElement {
    const found = host.querySelector<HTMLButtonElement>(selector);
    expect(found).not.toBeNull();
    return found!;
  }

  function setPageOffsets(host: HTMLElement) {
    [...host.querySelectorAll<HTMLElement>(".pdf-page")].forEach((element, index) => {
      Object.defineProperty(element, "offsetTop", { configurable: true, value: (index + 1) * 100 });
    });
  }

  it("validates the local theme, exposes all choices, and persists presentation-only changes for later mounts", async () => {
    localStorage.setItem("ls-pdf-viewer-theme", "graph-dark");
    const firstPage = page(612, 792);
    const firstPdf = documentWithPages([firstPage]);
    getDocumentMock.mockReturnValueOnce({ promise: Promise.resolve(firstPdf) });
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    const writeHighlights = vi.spyOn(backend(), "writeHighlights").mockResolvedValue(undefined);
    const writeViewState = vi.spyOn(backend() as any, "writePdfViewState").mockResolvedValue(undefined);
    const writeText = vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    const saveArea = vi.spyOn(backend(), "savePdfAreaImage").mockResolvedValue("");
    const first = mountViewer();
    try {
      await flush();
      const viewer = first.host.querySelector(".pdf-viewer")!;
      expect(viewer.getAttribute("data-theme")).toBe("light");

      TestIntersectionObserver.instances[0].show(first.host.querySelector(".pdf-page")!);
      await flush();
      expect(firstPage.render).toHaveBeenCalledOnce();
      const getPageCallsBeforeThemes = firstPdf.getPage.mock.calls.length;

      button(first.host, 'button[title="More settings"]').click();
      const choices = [...first.host.querySelectorAll<HTMLButtonElement>(".pdf-theme-choice")];
      expect(choices.map((choice) => choice.getAttribute("aria-label"))).toEqual([
        "Light PDF theme",
        "Warm PDF theme",
        "Dark PDF theme",
      ]);
      for (const theme of ["warm", "dark", "light", "dark"] as const) {
        button(first.host, `button[aria-label="${theme[0].toUpperCase()}${theme.slice(1)} PDF theme"]`).click();
        await flush();
        expect(viewer.getAttribute("data-theme")).toBe(theme);
        expect(localStorage.getItem("ls-pdf-viewer-theme")).toBe(theme);
      }
      expect(firstPage.render).toHaveBeenCalledOnce();
      expect(firstPdf.getPage).toHaveBeenCalledTimes(getPageCallsBeforeThemes);
      expect(writeHighlights).not.toHaveBeenCalled();
      expect(writeViewState).not.toHaveBeenCalled();
      expect(writeText).not.toHaveBeenCalled();
      expect(saveArea).not.toHaveBeenCalled();
    } finally {
      first.dispose();
    }

    const secondPdf = documentWithPages([page(612, 792)]);
    getDocumentMock.mockReturnValueOnce({ promise: Promise.resolve(secondPdf) });
    const second = mountViewer("later.pdf");
    try {
      await flush();
      expect(second.host.querySelector(".pdf-viewer")?.getAttribute("data-theme")).toBe("dark");
    } finally {
      second.dispose();
    }
  });

  it("loads an empty outline once without blocking first paint", async () => {
    let resolveOutline!: (items: unknown[]) => void;
    const pendingOutline = new Promise<unknown[]>((resolve) => { resolveOutline = resolve; });
    const pdf = documentWithPages([page(612, 792)]);
    pdf.getOutline.mockReturnValue(pendingOutline);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(pdf) });
    const view = mountViewer();
    try {
      await flush();
      expect(view.host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready")).toBe("true");
      button(view.host, 'button[title="Outline"]').click();
      button(view.host, 'button[title="Outline"]').click();
      button(view.host, 'button[title="Outline"]').click();
      expect(pdf.getOutline).toHaveBeenCalledOnce();

      resolveOutline([]);
      await flush();
      expect(view.host.querySelector(".pdf-outline-empty")?.textContent).toBe("No outlines");
    } finally {
      view.dispose();
    }
  });

  it("keeps nested items collapsed, separates disclosure from labels, and resolves named, integer, and ref destinations", async () => {
    const ref = { num: 17, gen: 0 };
    const pdf = documentWithPages(Array.from({ length: 4 }, () => page(612, 792)));
    pdf.getOutline.mockResolvedValue([
      {
        title: "<img src=x onerror=alert(1)>",
        dest: "chapter-two",
        url: "https://example.invalid/must-not-open",
        items: [{ title: "Integer page", dest: [2, { name: "XYZ" }], items: [] }],
      },
      { title: "Reference page", dest: [ref, { name: "Fit" }], items: [] },
      { title: "URL only", dest: null, url: "https://example.invalid/never", items: [] },
    ] as any);
    pdf.getDestination.mockResolvedValue([1, { name: "Fit" }]);
    pdf.getPageIndex.mockResolvedValue(3);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(pdf) });
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    const view = mountViewer();
    try {
      await flush();
      setPageOffsets(view.host);
      button(view.host, 'button[title="Outline"]').click();
      await flush();

      const labels = [...view.host.querySelectorAll<HTMLButtonElement>(".pdf-outline-label")];
      expect(labels.map((label) => label.textContent)).toEqual([
        "<img src=x onerror=alert(1)>",
        "Reference page",
        "URL only",
      ]);
      expect(view.host.querySelector(".pdf-outline-label img")).toBeNull();
      expect(view.host.querySelector(".pdf-outline-children")).toBeNull();

      const disclosure = button(view.host, ".pdf-outline-disclosure");
      disclosure.click();
      await flush();
      expect(pdf.getDestination).not.toHaveBeenCalled();
      expect(view.host.querySelector(".pdf-outline-children")).not.toBeNull();
      expect(disclosure.getAttribute("aria-expanded")).toBe("true");

      const scroll = view.host.querySelector<HTMLElement>(".pdf-scroll")!;
      button(view.host, ".pdf-outline-label").click();
      await flush();
      expect(pdf.getDestination).toHaveBeenCalledWith("chapter-two");
      expect(scroll.scrollTop).toBe(200);
      expect(disclosure.getAttribute("aria-expanded")).toBe("true");

      button(view.host, ".pdf-outline-children .pdf-outline-label").click();
      await flush();
      expect(scroll.scrollTop).toBe(300);
      expect(pdf.getPageIndex).not.toHaveBeenCalled();

      labels[1].click();
      await flush();
      expect(pdf.getPageIndex).toHaveBeenCalledWith(ref);
      expect(scroll.scrollTop).toBe(400);

      labels[2].click();
      await flush();
      expect(open).not.toHaveBeenCalled();
      expect(scroll.scrollTop).toBe(400);
    } finally {
      view.dispose();
    }
  });

  it("clears stale outline state on document identity teardown", async () => {
    let resolveFirst!: (items: unknown[]) => void;
    const firstOutline = new Promise<unknown[]>((resolve) => { resolveFirst = resolve; });
    const first = documentWithPages([page(612, 792)]);
    first.getOutline.mockReturnValue(firstOutline);
    const second = documentWithPages([page(612, 792)]);
    second.getOutline.mockResolvedValue([{ title: "Current document", dest: [0], items: [] }] as any);
    getDocumentMock
      .mockReturnValueOnce({ promise: Promise.resolve(first) })
      .mockReturnValueOnce({ promise: Promise.resolve(second) });
    const [target, setTarget] = createSignal({ filename: "a.pdf", label: "A" });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <KeyedPdfViewer target={target} />, host);
    try {
      await flush();
      setTarget({ filename: "b.pdf", label: "B" });
      await flush();
      resolveFirst([{ title: "Stale document", dest: [0], items: [] }]);
      await flush();
      button(host, 'button[title="Outline"]').click();
      await flush();

      expect(first.getOutline).toHaveBeenCalledOnce();
      expect(second.getOutline).toHaveBeenCalledOnce();
      expect(host.querySelector(".pdf-outline-panel")?.textContent).toContain("Current document");
      expect(host.querySelector(".pdf-outline-panel")?.textContent).not.toContain("Stale document");
    } finally {
      dispose();
    }
  });

  it("dismisses only settings or outline on outside pointer and Escape", async () => {
    const pdf = documentWithPages([page(612, 792)]);
    pdf.getOutline.mockResolvedValue([{ title: "Chapter", dest: [0], items: [] }] as any);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(pdf) });
    const view = mountViewer();
    try {
      await flush();
      button(view.host, 'button[title="More settings"]').click();
      expect(view.host.querySelector(".pdf-settings-menu")).not.toBeNull();
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      await flush();
      expect(view.host.querySelector(".pdf-settings-menu")).toBeNull();
      expect(view.host.querySelector(".pdf-viewer")).not.toBeNull();

      button(view.host, 'button[title="Outline"]').click();
      expect(view.host.querySelector(".pdf-outline-panel")).not.toBeNull();
      expect(dismissTopTransient("escape")).toBe(true);
      await flush();
      expect(view.host.querySelector(".pdf-outline-panel")).toBeNull();
      expect(view.host.querySelector(".pdf-viewer")).not.toBeNull();
      expect(pdf.destroy).not.toHaveBeenCalled();
    } finally {
      view.dispose();
    }
  });

  it("styles light, warm, and dark locally while leaving the highlight layer uninverted", () => {
    const css = readFileSync("src/styles/app.css", "utf8");
    expect(css).toContain('.pdf-viewer[data-theme="light"] {\n  --pdf-container-bg: #fff;');
    expect(css).toContain('.pdf-viewer[data-theme="warm"] {\n  --pdf-container-bg: #f6efdf;');
    expect(css).toContain('  --pdf-toolbar-bg: #f6efdf;\n  --pdf-page-bg: #f8eeda;');
    expect(css).toContain('.pdf-viewer[data-theme="dark"] {\n  --pdf-container-bg: #202124;');
    expect(css).toMatch(/\.pdf-viewer\[data-theme="warm"\] \.pdf-page > :is\(canvas, \.textLayer\) \{[^}]*filter: sepia/s);
    expect(css).toMatch(/\.pdf-viewer\[data-theme="dark"\] \.pdf-page > :is\(canvas, \.textLayer\) \{[^}]*filter: invert\(1\) hue-rotate\(180deg\)/s);
    expect(css).toMatch(/\.pdf-viewer\[data-theme="dark"\] \.pdf-hl \{[^}]*mix-blend-mode: screen/s);
    expect(css).not.toMatch(/\.pdf-viewer\[data-theme="dark"\] \.pdf-hl-layer \{[^}]*filter:/s);
  });
});
