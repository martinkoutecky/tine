import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { backend } from "../backend";
import { KeyedPdfViewer } from "./PdfViewer";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";
import {
  clearTransientLayersForTest,
  dismissTopTransient,
  topTransientLayer,
} from "../transientLayers";
import type { PdfTarget } from "../ui";

vi.mock("../nativeChrome", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../nativeChrome")>()),
  isMobilePlatform: true,
}));

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
  for (let i = 0; i < 16; i++) await Promise.resolve();
}

class TestIntersectionObserver {
  constructor(private readonly callback: IntersectionObserverCallback) {}

  observe(target: Element) {
    this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
  unobserve() {}
  disconnect() {}
  takeRecords() { return []; }
}

function documentWithOnePage() {
  return {
    numPages: 1,
    getPage: vi.fn().mockResolvedValue({
      getViewport: vi.fn(({ scale }: { scale: number }) => ({ width: 612 * scale, height: 792 * scale })),
      getTextContent: vi.fn().mockResolvedValue({ items: [] }),
      render: vi.fn().mockReturnValue({ promise: Promise.resolve(), cancel: vi.fn() }),
    }),
    getOutline: vi.fn().mockResolvedValue([]),
    getDestination: vi.fn().mockResolvedValue(null),
    getPageIndex: vi.fn().mockResolvedValue(0),
    destroy: vi.fn().mockResolvedValue(undefined),
  };
}

describe("mobile PDF pane transient ownership", () => {
  beforeEach(() => {
    clearTransientLayersForTest();
    resetPdfOwnershipForTest();
    getDocumentMock.mockReset();
    vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: vi.fn(),
    });
    vi.spyOn(backend() as any, "openPdf").mockResolvedValue({
      highlights: [],
      page: 1,
      scale: 1,
    });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(documentWithOnePage()) });
  });

  afterEach(() => {
    clearTransientLayersForTest();
    resetPdfOwnershipForTest();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.replaceChildren();
    Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
  });

  it("owns Back/Escape while the mobile pane takes over, after its inner Find layer", async () => {
    const owner = activatePdfOwnership("/test/mobile-pdf");
    const [target, setTarget] = createSignal<PdfTarget | null>({ filename: "mobile.pdf", label: "Mobile PDF", owner });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <KeyedPdfViewer target={target} onClose={() => setTarget(null)} />, host);
    try {
      await flush();
      expect(topTransientLayer()?.id).toMatch(/^pdf-viewer-.*-surface$/);

      (host.querySelector('button[title="Find in document (Ctrl+F)"]') as HTMLButtonElement).click();
      await flush();
      expect(dismissTopTransient("back")).toBe(true);
      expect(target()).not.toBeNull();

      (host.querySelector('button[title="More settings"]') as HTMLButtonElement).click();
      await flush();
      expect([...host.querySelectorAll(".pdf-settings-overflow button")].map((button) => button.textContent?.trim())).toEqual([
        "Fit width",
        "Fit height",
        "Area highlight",
        "Notes",
        "Outline",
      ]);
      expect(dismissTopTransient("back")).toBe(true);
      expect(target()).not.toBeNull();

      expect(dismissTopTransient("escape")).toBe(true);
      await flush();
      expect(target()).toBeNull();
      expect(topTransientLayer()).toBeUndefined();
    } finally {
      dispose();
    }
  });

  it("turns a completed touch text selection into the annotation color chooser", async () => {
    const writeHighlights = vi.spyOn(backend(), "writeHighlights").mockResolvedValue(undefined);
    vi.spyOn(backend(), "writeText").mockResolvedValue(undefined);
    const owner = activatePdfOwnership("/test/mobile-pdf-selection");
    const [target] = createSignal<PdfTarget | null>({ filename: "mobile.pdf", label: "Mobile PDF", owner });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <KeyedPdfViewer target={target} />, host);
    try {
      await flush();
      const page = host.querySelector<HTMLElement>(".pdf-page")!;
      const textLayer = page.querySelector<HTMLElement>(".textLayer")!;
      const span = document.createElement("span");
      const text = document.createTextNode("select this text");
      span.appendChild(text);
      textLayer.appendChild(span);
      vi.spyOn(page, "getBoundingClientRect").mockReturnValue({
        left: 0, top: 0, right: 612, bottom: 792, width: 612, height: 792, x: 0, y: 0,
        toJSON: () => ({}),
      });
      vi.spyOn(window, "getSelection").mockReturnValue({
        isCollapsed: false,
        toString: () => "select this text",
        getRangeAt: () => ({
          commonAncestorContainer: text,
          getClientRects: () => [{ left: 18, top: 24, right: 130, bottom: 40, width: 112, height: 16 }],
        }),
        removeAllRanges: vi.fn(),
      } as unknown as Selection);

      span.dispatchEvent(new Event("touchend", { bubbles: true }));
      await flush();

      expect(host.querySelector(".pdf-color-menu")).not.toBeNull();
      expect(host.querySelectorAll(".pdf-color-swatch")).toHaveLength(5);
      host.querySelector(".pdf-color-swatch")!.dispatchEvent(new Event("pointerdown", {
        bubbles: true,
        cancelable: true,
      }));
      await flush();
      expect(writeHighlights).toHaveBeenCalledOnce();
      expect(writeHighlights.mock.calls[0][2][0].text).toBe("select this text");
    } finally {
      dispose();
    }
  });

  it("opens the complete annotation actions from a mobile long press on an existing highlight", async () => {
    const id = "11111111-1111-4111-8111-111111111111";
    vi.mocked(backend().openPdf).mockResolvedValue({
      highlights: [{
        id,
        page: 1,
        position: {
          page: 1,
          bounding: { left: 20, top: 40, width: 100, height: 14 },
          rects: [{ left: 20, top: 40, width: 100, height: 14 }],
        },
        color: "yellow",
        text: "existing annotation",
        image: null,
      }],
      page: 1,
      scale: 1,
    });
    const owner = activatePdfOwnership("/test/mobile-pdf-highlight");
    const [target] = createSignal<PdfTarget | null>({ filename: "mobile.pdf", label: "Mobile PDF", owner });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <KeyedPdfViewer target={target} />, host);
    try {
      await flush();
      const highlight = host.querySelector<HTMLElement>(`[data-highlight-id="${id}"]`)!;
      expect(highlight).not.toBeNull();
      const longPress = new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: 40,
        clientY: 60,
      });
      highlight.dispatchEvent(longPress);
      await flush();

      expect(longPress.defaultPrevented).toBe(true);
      const labels = [...host.querySelectorAll<HTMLButtonElement>(".pdf-color-menu button")]
        .map((button) => button.textContent?.trim())
        .filter(Boolean);
      expect(labels).toEqual(expect.arrayContaining(["Copy ref", "Linked references", "✕"]));
    } finally {
      dispose();
    }
  });
});
