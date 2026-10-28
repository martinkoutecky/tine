import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";
import { PdfViewer } from "./PdfViewer";

const { getDocument } = vi.hoisted(() => ({ getDocument: vi.fn() }));
vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: {}, getDocument,
  TextLayer: class {
    render() { return Promise.resolve(); }
    update() { return Promise.resolve(); }
    cancel() {}
  },
}));
vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({ default: "worker.js" }));

class Visibility {
  static observers: Visibility[] = [];
  constructor(private callback: IntersectionObserverCallback) { Visibility.observers.push(this); }
  observe() {}
  disconnect() {}
  change(target: Element, isIntersecting: boolean) {
    this.callback([{ target, isIntersecting } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
}

async function flush() { for (let n = 0; n < 80; n++) await Promise.resolve(); }

function setup() {
  // Three full-size DPR-2 pages fill the desktop pixel budget; later visible
  // pages must evict old decoded data as well as their canvas backing stores.
  const pages = Array.from({ length: 10 }, () => ({
    getViewport: ({ scale }: { scale: number }) => ({ width: 2000 * scale, height: 2000 * scale }),
    getTextContent: vi.fn().mockResolvedValue({ items: [] }),
    render: vi.fn(() => ({ promise: Promise.resolve(), cancel: vi.fn() })),
    cleanup: vi.fn(() => true),
  }));
  const doc = { numPages: pages.length, getPage: vi.fn(async (n: number) => pages[n - 1]),
    getOutline: vi.fn().mockResolvedValue([]), destroy: vi.fn().mockResolvedValue(undefined) };
  getDocument.mockReturnValue({ promise: Promise.resolve(doc) });
  const owner = activatePdfOwnership("/test/pdf-cleanup");
  function mount() {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PdfViewer owner={owner} filename="scan.pdf" label="Scan" />, host);
    return { host, dispose };
  }
  return { pages, mount };
}

beforeEach(() => {
  Visibility.observers = [];
  vi.stubGlobal("IntersectionObserver", Visibility);
  vi.stubGlobal("devicePixelRatio", 2);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
  vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
  vi.spyOn(backend(), "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: vi.fn() });
});
afterEach(() => {
  resetPdfOwnershipForTest();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  document.body.replaceChildren();
  Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
  Reflect.deleteProperty(document, "elementFromPoint");
});

it("releases decoded data on eviction and renders selectable content again on scroll-back without graph writes", async () => {
  const { pages, mount } = setup();
  const id = "11111111-1111-4111-8111-111111111111";
  const rect = { top: 40, left: 20, width: 80, height: 12 };
  vi.mocked(backend().openPdf).mockResolvedValue({
    page: 1, scale: 1,
    highlights: [{ id, page: 1, color: "yellow", text: "Keep this annotation", image: null,
      position: { page: 1, bounding: rect, rects: [rect] } }],
  });
  const writes = vi.spyOn(backend(), "writeHighlights");
  const crops = vi.spyOn(backend(), "savePdfAreaImage");
  const view = mount();
  try {
    await flush();
    const wraps = view.host.querySelectorAll(".pdf-page");
    const visibility = Visibility.observers[0];
    visibility.change(wraps[0], true);
    await flush();
    expect(wraps[0].querySelector("canvas")).not.toBeNull();
    expect(wraps[0].querySelector(`[data-highlight-id="${id}"]`)).not.toBeNull();
    visibility.change(wraps[0], false);
    visibility.change(wraps[5], true);
    await flush();
    expect(wraps[0].querySelector("canvas")).toBeNull();
    expect(pages[0].cleanup).toHaveBeenCalledOnce();
    const previousRenders = pages[0].render.mock.calls.length;
    const previousText = pages[0].getTextContent.mock.calls.length;
    visibility.change(wraps[5], false);
    visibility.change(wraps[0], true);
    await flush();
    expect(wraps[0].querySelector("canvas")).not.toBeNull();
    expect(pages[0].render.mock.calls.length).toBeGreaterThan(previousRenders);
    expect(pages[0].getTextContent.mock.calls.length).toBeGreaterThan(previousText);
    expect(wraps[0].querySelector(`[data-highlight-id="${id}"]`)).not.toBeNull();
    expect(writes).not.toHaveBeenCalled();
    expect(crops).not.toHaveBeenCalled();
  } finally { view.dispose(); }
});

it("keeps shared decoded data until the last retaining split view releases the page", async () => {
  const { pages, mount } = setup();
  const first = mount();
  const second = mount();
  let secondClosed = false;
  try {
    await flush();
    const firstPages = first.host.querySelectorAll(".pdf-page");
    const secondPage = second.host.querySelector(".pdf-page")!;
    Visibility.observers[0].change(firstPages[0], true);
    Visibility.observers[1].change(secondPage, true);
    await flush();
    Visibility.observers[0].change(firstPages[0], false);
    Visibility.observers[0].change(firstPages[5], true);
    await flush();
    expect(firstPages[0].querySelector("canvas")).toBeNull();
    expect(secondPage.querySelector("canvas")).not.toBeNull();
    expect(pages[0].cleanup).not.toHaveBeenCalled();
    second.dispose();
    secondClosed = true;
    expect(pages[0].cleanup).toHaveBeenCalledOnce();
  } finally {
    first.dispose();
    if (!secondClosed) second.dispose();
  }
});
