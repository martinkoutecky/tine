import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { activatePdfOwnership, resetPdfOwnershipForTest, retirePdfOwnership, type PdfOwnership } from "../pdfOwnership";
import { makePdfRoute, type PdfRoute } from "../router";
import { setToasts, toasts } from "../toasts";
import { KeyedPdfViewer } from "./KeyedPdfViewer";

const getDocument = vi.hoisted(() => vi.fn());
vi.mock("pdfjs-dist", () => ({ GlobalWorkerOptions: {}, getDocument }));
vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({ default: "test-worker" }));

async function flush() {
  for (let i = 0; i < 24; i++) await Promise.resolve();
}

function pendingOpen() {
  let resolve!: (value: { highlights: []; page: number; scale: number }) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<{ highlights: []; page: number; scale: number }>((yes, no) => {
    resolve = yes; reject = no;
  });
  return { promise, resolve, reject };
}

describe("PDF annotation loading ownership (GH #557)", () => {
  beforeEach(() => {
    resetPdfOwnershipForTest();
    setToasts([]);
    vi.stubGlobal("IntersectionObserver", class {
      observe() {} disconnect() {} unobserve() {}
    });
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
    vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
    getDocument.mockReset();
    getDocument.mockImplementation(() => ({ promise: Promise.resolve({
      numPages: 1,
      getPage: vi.fn().mockResolvedValue({ getViewport: () => ({ width: 612, height: 792 }) }),
      getOutline: vi.fn().mockResolvedValue([]),
      destroy: vi.fn().mockResolvedValue(undefined),
    }) }));
  });
  afterEach(() => {
    resetPdfOwnershipForTest();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
    document.body.replaceChildren();
    setToasts([]);
  });

  function mount(owner: PdfOwnership = activatePdfOwnership("/test/qe2")) {
    const [route, setRoute] = createSignal<PdfRoute | null>(makePdfRoute("a.pdf", "A"));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <KeyedPdfViewer route={route} owner={() => owner} />, host);
    return { host, setRoute, dispose, owner };
  }

  it.each(["success", "failure"])("drops delayed %s after close and preserves a reopened reader", async (outcome) => {
    const pending = pendingOpen();
    const open = vi.spyOn(backend(), "openPdf")
      .mockReturnValueOnce(pending.promise)
      .mockResolvedValue({ highlights: [], page: 1, scale: 1 });
    const view = mount();
    try {
      await flush();
      expect(open).toHaveBeenCalledOnce();
      view.setRoute(null);
      view.setRoute(makePdfRoute("a.pdf", "Reopened A"));
      await flush();
      if (outcome === "failure") pending.reject(new Error("annotation read failed"));
      else pending.resolve({ highlights: [], page: 3, scale: 2 });
      await flush();
      expect(toasts()).toEqual([]);
      expect(backend().readAsset).toHaveBeenCalledOnce();
      expect(view.host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready")).toBe("true");
      expect((view.host.querySelector(".pdf-page-input") as HTMLInputElement).value).toBe("1");
    } finally { view.dispose(); }
  });

  it.each(["close", "switch", "reopen", "graph retirement"])("drops a failure if %s lands between read rejection and the viewer catch", async (action) => {
    const pending = pendingOpen();
    vi.spyOn(backend(), "openPdf").mockReturnValueOnce(pending.promise)
      .mockResolvedValue({ highlights: [], page: 1, scale: 1 });
    const view = mount();
    try {
      await flush();
      pending.reject(new Error("annotation read failed"));
      // The read wrapper observes the rejection, then teardown runs before
      // the awaiting viewer's catch continuation. Both are ordinary microtasks.
      queueMicrotask(() => {
        if (action === "graph retirement") retirePdfOwnership();
        view.setRoute(null);
        if (action === "switch" || action === "reopen") {
          view.setRoute(makePdfRoute(action === "switch" ? "b.pdf" : "a.pdf", "Next"));
        }
      });
      await flush();
      expect(toasts()).toEqual([]);
      if (action === "switch" || action === "reopen") {
        expect(view.host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready")).toBe("true");
        expect(view.host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-filename"))
          .toBe(action === "switch" ? "b.pdf" : "a.pdf");
        expect(backend().readAsset).toHaveBeenCalledOnce();
      } else {
        expect(view.host.querySelector(".pdf-viewer")).toBeNull();
        expect(backend().readAsset).not.toHaveBeenCalled();
      }
    } finally { view.dispose(); }
  });

  it("retiring one same-asset pane keeps the other pane's annotation error visible", async () => {
    const firstRead = pendingOpen(), secondRead = pendingOpen();
    vi.spyOn(backend(), "openPdf").mockReturnValueOnce(firstRead.promise).mockReturnValueOnce(secondRead.promise);
    const first = mount(), second = mount(first.owner);
    try {
      await flush();
      firstRead.reject(new Error("closed pane"));
      queueMicrotask(() => first.setRoute(null));
      await flush();
      secondRead.reject(new Error("active pane"));
      await flush();
      expect(toasts().map((toast) => toast.message)).toEqual([
        "Couldn't load PDF annotations. (Error: active pane)",
      ]);
      expect(first.host.querySelector(".pdf-viewer")).toBeNull();
      expect(second.host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready")).toBe("true");
      expect(backend().readAsset).toHaveBeenCalledOnce();
    } finally { first.dispose(); second.dispose(); }
  });

  it("reports a current annotation failure and still displays the PDF", async () => {
    vi.spyOn(backend(), "openPdf").mockRejectedValue(new Error("malformed sidecar"));
    const view = mount();
    try {
      await flush();
      expect(toasts().map((toast) => toast.message)).toEqual([
        "Couldn't load PDF annotations. (Error: malformed sidecar)",
      ]);
      expect(view.host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready")).toBe("true");
    } finally { view.dispose(); }
  });
});
