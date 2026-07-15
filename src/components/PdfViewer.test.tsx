import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import {
  KeyedPdfViewer,
  PdfViewer,
  PDF_CANVAS_CACHE_PIXEL_BUDGET,
  PDF_FIND_MATCH_CAP,
} from "./PdfViewer";

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
