import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Show, createSignal } from "solid-js";
import { backend } from "../backend";
import { KeyedPdfViewer } from "./PdfViewer";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";
import { clearTransientLayersForTest } from "../transientLayers";
import { makePdfRoute, type PdfRoute } from "../router";
import { installTouchMedia, setTouchInput } from "../tests/touchMediaTestkit";

// Review r1 of D-20261018-W1: the PDF viewer registered its touch
// `selectionchange` listener once at mount. It now follows the capability, so a
// touch input that appears (or goes away) after the PDF opened changes whether
// a long-press selection offers the highlight colours, with no leaked listener.
const getDocumentMock = vi.hoisted(() => vi.fn());
vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: {},
  getDocument: getDocumentMock,
  TextLayer: class {
    render() { return Promise.resolve(); }
    update() { return Promise.resolve(); }
  },
}));
vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({ default: "pdf.worker.test.js" }));

async function flush() {
  for (let i = 0; i < 16; i++) await Promise.resolve();
}
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 200)); // the 120 ms debounce

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
      cleanup: vi.fn(() => true),
    }),
    getOutline: vi.fn().mockResolvedValue([]),
    getDestination: vi.fn().mockResolvedValue(null),
    getPageIndex: vi.fn().mockResolvedValue(0),
    destroy: vi.fn().mockResolvedValue(undefined),
  };
}

beforeAll(() => installTouchMedia());
beforeEach(() => {
  setTouchInput(false);
  clearTransientLayersForTest();
  resetPdfOwnershipForTest();
  getDocumentMock.mockReset();
  vi.stubGlobal("IntersectionObserver", TestIntersectionObserver);
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
  vi.spyOn(backend() as any, "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
  vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
  getDocumentMock.mockReturnValue({ promise: Promise.resolve(documentWithOnePage()) });
});
afterEach(() => {
  clearTransientLayersForTest();
  resetPdfOwnershipForTest();
  setTouchInput(false);
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  document.body.replaceChildren();
  Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
});

async function mountViewer() {
  const owner = activatePdfOwnership("/test/pdf-touch-rewire");
  const [route, setRoute] = createSignal<PdfRoute | null>(makePdfRoute("rewire.pdf", "Rewire PDF"));
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <Show when={route()}>{(r) =>
    <KeyedPdfViewer route={() => r()} owner={() => owner} onClose={() => setRoute(null)} />
  }</Show>, host);
  await flush();
  return { host, dispose };
}

/** A native text selection inside the page's text layer, as a long press makes it. */
function selectPdfText(host: HTMLElement) {
  const page = host.querySelector<HTMLElement>(".pdf-page")!;
  const layer = page.querySelector<HTMLElement>(".textLayer")!;
  const span = document.createElement("span");
  const text = document.createTextNode("select this text");
  span.appendChild(text);
  layer.appendChild(span);
  vi.spyOn(page, "getBoundingClientRect").mockReturnValue({
    left: 0, top: 0, right: 612, bottom: 792, width: 612, height: 792, x: 0, y: 0, toJSON: () => ({}),
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
  return span;
}
const swatches = (host: HTMLElement) => host.querySelectorAll(".pdf-color-swatch").length;
const selectionChangeListeners = (spy: ReturnType<typeof vi.spyOn>) =>
  spy.mock.calls.filter(([type]) => type === "selectionchange").map(([, fn]) => fn);

function touchEvent(el: Element, type: string, points: [number, number, number][]) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, "touches", { value: points.map(([identifier, clientX, clientY]) =>
    ({ identifier, clientX, clientY })) });
  el.dispatchEvent(event);
  return event;
}

it("previews a touch pinch without rasterizing, then commits exactly one shared zoom", async () => {
  const { host, dispose } = await mountViewer();
  try {
    const scroll = host.querySelector<HTMLElement>(".pdf-scroll")!;
    const page = host.querySelector<HTMLElement>(".pdf-page")!;
    vi.spyOn(page, "getBoundingClientRect").mockImplementation(() => {
      const scale = Number(scroll.style.getPropertyValue("--scale-factor"));
      return { left: 20 - scroll.scrollLeft, top: 40 - scroll.scrollTop, width: 612 * scale,
        height: 792 * scale, right: 20 - scroll.scrollLeft + 612 * scale,
        bottom: 40 - scroll.scrollTop + 792 * scale } as DOMRect;
    });
    const doc = await getDocumentMock.mock.results[0].value.promise;
    const pdfPage = await doc.getPage(1);
    const renders = pdfPage.render.mock.calls.length;
    const before = host.querySelector(".pdf-zoom-level")!.textContent;
    touchEvent(scroll, "touchstart", [[1, 100, 100]]);
    const start = touchEvent(scroll, "touchstart", [[1, 100, 100], [2, 200, 100]]);
    expect(start.defaultPrevented).toBe(true);
    touchEvent(scroll, "touchmove", [[1, 60, 110], [2, 260, 110]]);
    expect(page.style.transform).toContain("scale(2)");
    expect(page.style.transform).toContain("translate(10px, 10px)");
    expect(host.querySelector(".pdf-zoom-level")!.textContent).toBe(before);
    expect(pdfPage.render).toHaveBeenCalledTimes(renders);
    touchEvent(scroll, "touchend", [[1, 50, 100]]);
    await flush();
    expect(host.querySelector(".pdf-zoom-level")!.textContent).toContain("200%");
    expect(page.style.transform).toBe("");
    expect(scroll.scrollLeft).toBeCloseTo(120);
    expect(scroll.scrollTop).toBeCloseTo(50);
    touchEvent(scroll, "touchend", []);
    await new Promise((resolve) => setTimeout(resolve, 260));
    await flush();
    expect(pdfPage.render).toHaveBeenCalledTimes(renders + 1);
  } finally { dispose(); }
});

it("cancels a pinch and suppresses its selection, while the next one-finger selection still works", async () => {
  setTouchInput(false, true);
  const { host, dispose } = await mountViewer();
  try {
    const span = selectPdfText(host);
    const scroll = host.querySelector<HTMLElement>(".pdf-scroll")!;
    const before = host.querySelector(".pdf-zoom-level")!.textContent;
    touchEvent(span, "touchstart", [[1, 100, 100], [2, 200, 100]]);
    touchEvent(span, "touchmove", [[1, 50, 100], [2, 250, 100]]);
    document.dispatchEvent(new Event("selectionchange"));
    await settle();
    expect(swatches(host)).toBe(0);
    touchEvent(span, "touchcancel", []);
    touchEvent(span, "touchend", []);
    await settle();
    expect(host.querySelector(".pdf-zoom-level")!.textContent).toBe(before);
    expect(swatches(host)).toBe(0);
    expect(scroll.classList.contains("pdf-pinching")).toBe(false);
    expect(touchEvent(span, "touchstart", [[3, 100, 100]]).defaultPrevented).toBe(false);
    touchEvent(span, "touchend", []);
    await flush();
    expect(swatches(host)).toBe(5);
  } finally { dispose(); }
});

describe("PDF touch selection follows the input capability (D-20261018-W1 review r1)", () => {
  it("offers the highlight colours once a touch input appears after the PDF opened", async () => {
    const { host, dispose } = await mountViewer();
    try {
      selectPdfText(host);
      document.dispatchEvent(new Event("selectionchange"));
      await settle();
      expect(swatches(host)).toBe(0);
      setTouchInput(false, true);
      document.dispatchEvent(new Event("selectionchange"));
      await settle();
      expect(swatches(host)).toBe(5);
    } finally { dispose(); }
  });

  it("stops offering them when the touch input goes away", async () => {
    setTouchInput(false, true);
    const { host, dispose } = await mountViewer();
    try {
      selectPdfText(host);
      setTouchInput(false);
      document.dispatchEvent(new Event("selectionchange"));
      await settle();
      expect(swatches(host)).toBe(0);
    } finally { dispose(); }
  });

  it("balances its selectionchange listeners across flips and unmount", async () => {
    setTouchInput(false, true);
    const { host, dispose } = await mountViewer();
    const add = vi.spyOn(document, "addEventListener");
    const remove = vi.spyOn(document, "removeEventListener");
    try {
      for (let i = 0; i < 3; i++) {
        setTouchInput(false);
        setTouchInput(false, true);
      }
      dispose();
      const added = selectionChangeListeners(add);
      const removed = selectionChangeListeners(remove);
      // Three re-attaches after the spies started; three detaches for the
      // flips and one for the unmount, each removing a listener that was added.
      expect(added).toHaveLength(3);
      expect(removed).toHaveLength(4);
      expect(new Set(removed).size).toBe(4);
      for (const fn of added) expect(removed).toContain(fn);
    } finally { host.remove(); }
  });
});
