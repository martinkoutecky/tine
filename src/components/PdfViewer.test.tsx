import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { PdfViewer } from "./PdfViewer";

const getDocumentMock = vi.hoisted(() => vi.fn());

vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: {},
  getDocument: getDocumentMock,
}));

vi.mock("pdfjs-dist/build/pdf.worker.min.mjs?url", () => ({
  default: "pdf.worker.test.js",
}));

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe("PdfViewer load failure", () => {
  beforeEach(() => {
    getDocumentMock.mockReset();
    vi.spyOn(backend(), "readHighlights").mockResolvedValue([]);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    document.body.replaceChildren();
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
});
