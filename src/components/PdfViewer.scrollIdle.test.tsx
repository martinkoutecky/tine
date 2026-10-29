import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";
import { PdfViewer } from "./PdfViewer";
import { KeyedPdfViewer } from "./KeyedPdfViewer";
import { createSignal } from "solid-js";
import { makePdfRoute } from "../router";
const { getDocument } = vi.hoisted(() => ({ getDocument: vi.fn() }));
vi.mock("pdfjs-dist", () => ({ GlobalWorkerOptions: {}, getDocument, TextLayer: class {
  render() { return Promise.resolve(); } update() { return Promise.resolve(); } cancel() {}
} }));
vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({ default: "worker.js" }));
afterEach(() => { vi.useRealTimers(); resetPdfOwnershipForTest(); vi.restoreAllMocks(); document.body.replaceChildren(); });
it("keeps the page indicator live but publishes only on idle, and flushes the final position on close", async () => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
  vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
  vi.spyOn(backend(), "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
  const page = { getViewport: ({ scale }: { scale: number }) => ({ width: 612 * scale, height: 792 * scale }),
    getTextContent: async () => ({ items: [] }), cleanup() {}, render: () => ({ promise: Promise.resolve(), cancel() {} }) };
  getDocument.mockReturnValue({ promise: Promise.resolve({ numPages: 5, getPage: async () => page,
    getOutline: async () => [], destroy: async () => {} }) });
  const owner = activatePdfOwnership("/test/scroll-idle"), onViewState = vi.fn();
  const writes = vi.spyOn(backend(), "writeHighlights");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PdfViewer owner={owner} filename="paper.pdf" label="Paper" onViewState={onViewState} />, host);
  try {
    for (let n = 0; n < 100; n++) await Promise.resolve();
    onViewState.mockClear(); vi.useFakeTimers();
    host.querySelector<HTMLButtonElement>('button[title="Zoom in"]')!.click();
    for (let n = 0; n < 100; n++) await Promise.resolve();
    expect(onViewState).toHaveBeenLastCalledWith({ page: 1, scale: 1.1 });
    host.querySelector<HTMLButtonElement>('button[title="Zoom out"]')!.click();
    await vi.advanceTimersByTimeAsync(250); onViewState.mockClear();
    const scroller = host.querySelector<HTMLElement>(".pdf-scroll")!;
    [...host.querySelectorAll<HTMLElement>(".pdf-page")].forEach((wrap, index) => {
      Object.defineProperty(wrap, "offsetTop", { configurable: true, value: index * 1000 });
    });
    for (const top of [1100, 2100, 3100, 3150]) {
      scroller.scrollTop = top; scroller.dispatchEvent(new Event("scroll"));
      await vi.advanceTimersByTimeAsync(50);
      expect(host.querySelector<HTMLInputElement>(".pdf-page-input")!.value).toBe(String(Math.floor(top / 1000) + 1));
      expect(onViewState).not.toHaveBeenCalled();
    }
    await vi.advanceTimersByTimeAsync(200);
    expect(onViewState.mock.calls).toEqual([[{ page: 4, scale: 1 }]]);
    scroller.scrollTop = 4100; scroller.dispatchEvent(new Event("scroll"));
    await vi.advanceTimersByTimeAsync(50); dispose();
    expect(onViewState).toHaveBeenLastCalledWith({ page: 5, scale: 1 });
    const count = onViewState.mock.calls.length;
    await vi.advanceTimersByTimeAsync(500);
    expect(onViewState).toHaveBeenCalledTimes(count); expect(writes).not.toHaveBeenCalled();
  } finally { dispose(); }
});
it("does not flush a retiring view's pending position into a newly selected PDF", async () => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({} as CanvasRenderingContext2D);
  vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
  vi.spyOn(backend(), "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
  const page = { getViewport: ({ scale }: { scale: number }) => ({ width: 612 * scale, height: 792 * scale }),
    getTextContent: async () => ({ items: [] }), cleanup() {}, render: () => ({ promise: Promise.resolve(), cancel() {} }) };
  getDocument.mockReturnValue({ promise: Promise.resolve({ numPages: 5, getPage: async () => page, getOutline: async () => [], destroy: async () => {} }) });
  const owner = activatePdfOwnership("/test/view-idle"), onViewState = vi.fn();
  const [route, setRoute] = createSignal(makePdfRoute("first.pdf", "First"));
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <KeyedPdfViewer owner={() => owner} route={route} onViewState={onViewState} />, host);
  try {
    for (let n = 0; n < 100; n++) await Promise.resolve();
    vi.useFakeTimers();
    [...host.querySelectorAll<HTMLElement>(".pdf-page")].forEach((wrap, index) => Object.defineProperty(wrap, "offsetTop", { configurable: true, value: index * 1000 }));
    const scroll = host.querySelector<HTMLElement>(".pdf-scroll")!;
    scroll.scrollTop = 2100; scroll.dispatchEvent(new Event("scroll")); await vi.advanceTimersByTimeAsync(50);
    onViewState.mockClear(); setRoute(makePdfRoute("second.pdf", "Second"));
    for (let n = 0; n < 100; n++) await Promise.resolve();
    await vi.advanceTimersByTimeAsync(500);
    expect(onViewState.mock.calls.some(([state]) => state.page === 3)).toBe(false);
    expect(onViewState).toHaveBeenLastCalledWith({ page: 1, scale: 1 });
  } finally { dispose(); }
});
