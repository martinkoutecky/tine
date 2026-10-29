import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";
import { PdfViewer } from "./PdfViewer";

const { getDocument } = vi.hoisted(() => ({ getDocument: vi.fn() }));
vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: {}, getDocument,
  PDFDataRangeTransport: class {
    constructor(public length: number) {}
    onDataRange = vi.fn();
  },
}));
vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({ default: "worker.js" }));

afterEach(() => {
  delete (backend() as { pdfAssetSource?: unknown }).pdfAssetSource;
  resetPdfOwnershipForTest();
  vi.restoreAllMocks();
  getDocument.mockReset();
  document.body.replaceChildren();
});

it("starts pdf.js from a range source without a whole-file read or annotation writes", async () => {
  const source = { length: 34_000_000, read: vi.fn().mockResolvedValue(new Uint8Array(65536)) };
  Object.assign(backend(), { pdfAssetSource: vi.fn().mockResolvedValue(source) });
  const whole = vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
  vi.spyOn(backend(), "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
  const writes = vi.spyOn(backend(), "writeHighlights");
  const crops = vi.spyOn(backend(), "savePdfAreaImage");
  const destroy = vi.fn().mockResolvedValue(undefined);
  getDocument.mockReturnValue({ promise: new Promise(() => {}), destroy });
  const owner = activatePdfOwnership("/test/range");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PdfViewer owner={owner} filename="scan.pdf" label="Scan" />, host);
  try {
    await vi.waitFor(() => expect(getDocument, host.textContent ?? "").toHaveBeenCalled());
    expect(whole).not.toHaveBeenCalled();
    const options = getDocument.mock.calls.at(-1)![0];
    expect(options.range.length).toBe(source.length);
    expect(options.data).toBeUndefined();
    options.range.requestDataRange(0, 65536);
    for (let n = 0; n < 20; n++) await Promise.resolve();
    expect(source.read).toHaveBeenCalledWith(0, 65536);
    expect(writes).not.toHaveBeenCalled(); expect(crops).not.toHaveBeenCalled();
  } finally { dispose(); }
  expect(destroy).toHaveBeenCalledOnce();
});

it("shows an IPC range failure and releases a loading task that never settles", async () => {
  Object.assign(backend(), { pdfAssetSource: vi.fn().mockResolvedValue({
    length: 34_000_000, read: vi.fn().mockRejectedValue(new Error("asset changed")),
  }) });
  vi.spyOn(backend(), "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
  const destroy = vi.fn().mockResolvedValue(undefined);
  getDocument.mockReturnValue({ promise: new Promise(() => {}), destroy });
  const owner = activatePdfOwnership("/test/range-error");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PdfViewer owner={owner} filename="scan.pdf" label="Scan" />, host);
  try {
    await vi.waitFor(() => expect(getDocument).toHaveBeenCalled());
    getDocument.mock.calls[0][0].range.requestDataRange(0, 65536);
    await vi.waitFor(() => expect(host.querySelector(".pdf-load-error")?.textContent).toContain("asset changed"));
    expect(destroy).toHaveBeenCalledOnce();
  } finally { dispose(); }
});

it.each([0, 268_435_457])("refuses unsafe range source length %s before creating a worker", async (length) => {
  const read = vi.fn();
  Object.assign(backend(), { pdfAssetSource: vi.fn().mockResolvedValue({ length, read }) });
  vi.spyOn(backend(), "openPdf").mockResolvedValue({ highlights: [], page: 1, scale: 1 });
  const owner = activatePdfOwnership("/test/range-length");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PdfViewer owner={owner} filename="scan.pdf" label="Scan" />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".pdf-load-error")).not.toBeNull());
    expect(getDocument).not.toHaveBeenCalled(); expect(read).not.toHaveBeenCalled();
  } finally { dispose(); }
});
