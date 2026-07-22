import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { KeyedPdfViewer } from "./PdfViewer";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";
import {
  clearTransientLayersForTest,
  dismissTopTransient,
  topTransientLayer,
} from "../transientLayers";
import { pdfTarget, setPdfTarget } from "../ui";

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
  constructor(_callback: IntersectionObserverCallback) {}

  observe() {}
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
    getDocumentMock.mockReturnValue({ promise: Promise.resolve(documentWithOnePage()) });
  });

  afterEach(() => {
    clearTransientLayersForTest();
    setPdfTarget(null);
    resetPdfOwnershipForTest();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.replaceChildren();
    Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
  });

  it("owns Back/Escape while the mobile pane takes over, after its inner Find layer", async () => {
    const owner = activatePdfOwnership("/test/mobile-pdf");
    setPdfTarget({ filename: "mobile.pdf", label: "Mobile PDF", owner });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <KeyedPdfViewer target={pdfTarget} />, host);
    try {
      await flush();
      expect(topTransientLayer()?.id).toBe("pdf-pane");

      (host.querySelector('button[title="Find in document (Ctrl+F)"]') as HTMLButtonElement).click();
      await flush();
      expect(dismissTopTransient("back")).toBe(true);
      expect(pdfTarget()).not.toBeNull();

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
      expect(pdfTarget()).not.toBeNull();

      expect(dismissTopTransient("escape")).toBe(true);
      await flush();
      expect(pdfTarget()).toBeNull();
      expect(topTransientLayer()).toBeUndefined();
    } finally {
      dispose();
    }
  });
});
