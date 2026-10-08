import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { activatePdfOwnership, currentPdfOwnership } from "../pdfOwnership";
import { PdfViewer as OwnedPdfViewer } from "./PdfViewer";

// UI-OG-C5-P6-TEXTLAYER: a pdf.js TextLayer render/update is owned work (I-20/I-21) and
// its failure is reported (I-9).

const h = vi.hoisted(() => {
  type Layer = {
    id: number; cancel: ReturnType<typeof vi.fn>; update: ReturnType<typeof vi.fn>;
    finish: () => void; fail: (error: Error) => void;
  };
  const layers: Layer[] = [];
  return { getDocument: vi.fn(), layers };
});

vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: {},
  getDocument: h.getDocument,
  TextLayer: class {
    layer: (typeof h.layers)[number];
    constructor() {
      let finish = () => {}, fail = (_error: Error) => {};
      const done = new Promise<void>((resolve, reject) => { finish = resolve; fail = reject; });
      done.catch(() => {});
      this.donePromise = done;
      this.layer = { id: h.layers.length, cancel: vi.fn(), update: vi.fn(async () => {}), finish, fail };
      h.layers.push(this.layer);
    }
    donePromise: Promise<void>;
    render() { return this.donePromise; }
    update(options: unknown) { return this.layer.update(options); }
    cancel() { this.layer.cancel(); }
  },
}));
vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({ default: "pdf.worker.test.js" }));

class Observer {
  static instances: Observer[] = [];
  constructor(private readonly callback: IntersectionObserverCallback) { Observer.instances.push(this); }
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() { return []; }
  show(element: Element) {
    this.callback([{ isIntersecting: true, target: element } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
}

function pdfPage() {
  return {
    getViewport: vi.fn(({ scale }: { scale: number }) => ({ width: 612 * scale, height: 792 * scale })),
    getTextContent: vi.fn().mockResolvedValue({ items: [] }),
    render: vi.fn().mockReturnValue({ promise: Promise.resolve(), cancel: vi.fn() }),
  };
}

async function flush() { for (let i = 0; i < 24; i++) await Promise.resolve(); }

async function mount() {
  vi.spyOn(backend() as any, "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
  vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
  vi.spyOn(backend(), "readHighlights").mockResolvedValue([]);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
  const page = pdfPage();
  h.getDocument.mockReturnValue({ promise: Promise.resolve({
    numPages: 1, getPage: vi.fn(async () => page), getOutline: vi.fn().mockResolvedValue([]),
    getDestination: vi.fn().mockResolvedValue(null), getPageIndex: vi.fn().mockResolvedValue(0),
    destroy: vi.fn().mockResolvedValue(undefined),
  }) });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const owner = currentPdfOwnership() ?? activatePdfOwnership("/test/pdf-graph");
  const dispose = render(() => <OwnedPdfViewer filename="t.pdf" label="T" owner={owner} />, host);
  await flush();
  Observer.instances.at(-1)!.show(host.querySelector(".pdf-page")!);
  await flush();
  return { host, dispose };
}

describe("PdfViewer text layer ownership", () => {
  beforeEach(() => {
    h.layers.length = 0;
    h.getDocument.mockReset();
    Observer.instances = [];
    vi.stubGlobal("IntersectionObserver", Observer);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.replaceChildren();
    Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
  });

  it("shows a failed text-layer render instead of leaving an unhandled rejection", async () => {
    const { host, dispose } = await mount();
    try {
      expect(h.layers).toHaveLength(1);
      h.layers[0].fail(new Error("glyph overflow"));
      await flush();
      expect(host.querySelector(".pdf-load-error")?.textContent).toContain("Couldn't draw this PDF text");
    } finally { dispose(); }
  });

  it("cancels a pending text-layer render when the viewer is retired", async () => {
    const { dispose } = await mount();
    expect(h.layers).toHaveLength(1);
    dispose();
    await flush();
    expect(h.layers[0].cancel).toHaveBeenCalled();
    h.layers[0].finish();
    await flush();
  });

  it("lets only the newest zoom build install its layer when an older render completes late", async () => {
    vi.useFakeTimers();
    const { host, dispose } = await mount();
    try {
      expect(h.layers).toHaveLength(1); // first build, still rendering
      (host.querySelector('button[title="Zoom in"]') as HTMLButtonElement).click();
      await vi.advanceTimersByTimeAsync(200);
      await flush();
      expect(h.layers.length, "the zoom re-raster starts a second build").toBe(2);
      expect(h.layers[0].cancel, "the superseded build is cancelled").toHaveBeenCalled();
      h.layers[1].finish();
      await flush();
      h.layers[0].finish(); // late completion of the superseded render
      await flush();
      (host.querySelector('button[title="Zoom in"]') as HTMLButtonElement).click();
      await vi.advanceTimersByTimeAsync(400);
      await flush();
      expect(h.layers[0].update, "a retired layer must not be repositioned").not.toHaveBeenCalled();
      expect(h.layers[1].update, "the installed layer is the one repositioned").toHaveBeenCalled();
    } finally { dispose(); }
  });
});
