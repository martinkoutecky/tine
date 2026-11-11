import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { authorizeExternalPdf, takeExternalPdfSource } from "../externalPdfSource";
import { invalidateBinding } from "../binding";
import { activatePdfOwnership, resetPdfOwnershipForTest } from "../pdfOwnership";
import { makePdfRoute } from "../router";
import { loadLocale, showLocale } from "../i18n";
import { PdfViewer } from "./PdfViewer";
import { KeyedPdfViewer } from "./KeyedPdfViewer";
import { pdfHighlightSessionCount } from "./pdfHighlightState";
import { InlineText } from "../render/inline";
import { initParser } from "../render/parse";
import * as panes from "../panes";
import { graphMeta, setGraphMeta } from "../graphSession";

const getDocument = vi.hoisted(() => vi.fn());
vi.mock("pdfjs-dist", () => ({ GlobalWorkerOptions: {}, getDocument }));
vi.mock("pdfjs-dist/legacy/build/pdf.worker.min.mjs?url", () => ({ default: "worker.js" }));
const filename = "file:///tmp/outside%20paper.pdf";

beforeEach(() => {
  vi.stubGlobal("IntersectionObserver", class { observe() {} disconnect() {} unobserve() {} });
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
  getDocument.mockReturnValue({ promise: Promise.resolve({
    numPages: 1,
    getPage: vi.fn().mockResolvedValue({
      getViewport: ({ scale }: { scale: number }) => ({ width: 612 * scale, height: 792 * scale }),
      getTextContent: vi.fn().mockResolvedValue({ items: [{ str: "Outside document searchableword" }] }),
      render: () => ({ promise: Promise.resolve(), cancel() {} }), cleanup: () => true,
    }),
    getOutline: vi.fn().mockResolvedValue([]),
    loadingTask: { destroy: vi.fn().mockResolvedValue(undefined) },
  }), destroy: vi.fn().mockResolvedValue(undefined) });
});
afterEach(() => {
  resetPdfOwnershipForTest(); invalidateBinding(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  getDocument.mockReset(); document.body.replaceChildren();
  Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView"); showLocale("en");
});

function mount(read?: () => Promise<Uint8Array>) {
  const owner = activatePdfOwnership("/synthetic/graph");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PdfViewer filename={filename} label="Outside paper" owner={owner} externalRead={read} />, host);
  return { host, dispose };
}

it("outside PDFs read only their explicit source, search and zoom without graph annotations", async () => {
  const annotation = vi.spyOn(backend(), "openPdf");
  const asset = vi.spyOn(backend(), "readAsset");
  const highlights = vi.spyOn(backend(), "readHighlights");
  const write = vi.spyOn(backend(), "writeHighlights");
  const crop = vi.spyOn(backend(), "savePdfAreaImage");
  const read = vi.fn().mockResolvedValue(new TextEncoder().encode("%PDF-1.7\n"));
  const count = pdfHighlightSessionCount();
  const { host, dispose } = mount(read);
  try {
    expect(annotation, "outside reads must never open graph annotation state").not.toHaveBeenCalled();
    await vi.waitFor(() => expect(host.querySelector(".pdf-viewer")?.getAttribute("data-pdf-ready")).toBe("true"));
    expect(read).toHaveBeenCalledOnce();
    expect(pdfHighlightSessionCount()).toBe(count);
    expect(host.textContent).toContain("read-only");
    expect(host.querySelector(".pdf-notes-btn")).toBeNull();
    expect(host.querySelector('[title^="Area highlight"]')).toBeNull();
    const buttons = [...host.querySelectorAll<HTMLButtonElement>("button")];
    buttons.find(button => button.title.includes("Find in document"))!.click();
    const find = host.querySelector<HTMLInputElement>(".pdf-find-input")!;
    find.value = "searchableword"; find.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await vi.waitFor(() => expect(host.querySelector(".pdf-find-count")?.textContent).toContain("1"));
    const scale = host.querySelector(".pdf-zoom-level")!.textContent;
    buttons.find(button => button.title === "Zoom in")!.click();
    expect(host.querySelector(".pdf-zoom-level")!.textContent).not.toBe(scale);
    const page = host.querySelector(".pdf-page")!;
    page.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, shiftKey: true }));
    page.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    expect(host.querySelector(".pdf-area-band, .pdf-color-menu")).toBeNull();
    buttons.find(button => button.getAttribute("aria-label") === "More settings")!.click();
    const settings = host.querySelector(".pdf-settings-menu")!;
    expect(settings.textContent).toContain("read-only");
    expect(settings.textContent).not.toContain("Area highlight");
    expect(settings.textContent).not.toContain("Notes");
    for (const spy of [annotation, asset, highlights, write, crop]) expect(spy).not.toHaveBeenCalled();
  } finally { dispose(); }
  expect(write).not.toHaveBeenCalled(); expect(crop).not.toHaveBeenCalled();
});

it.each([undefined, "permission denied", "not a PDF", "larger than 256 MiB"])(
  "shows a closable error with its path for %s", async error => {
    const read = error === undefined ? undefined : vi.fn().mockRejectedValue(new Error(error));
    const { host, dispose } = mount(read);
    try {
      await vi.waitFor(() => expect(host.querySelector(".pdf-load-error")).not.toBeNull());
      expect(host.querySelector(".pdf-load-error")?.textContent).toContain("/tmp/outside paper.pdf");
      expect(host.querySelector(".pdf-load-error")?.textContent).toContain(error ?? "Open the original PDF link again");
      expect(host.querySelector(".pdf-close-btn")).not.toBeNull();
      expect(getDocument).not.toHaveBeenCalled();
    } finally { dispose(); }
  });

it("a keyed viewer consumes its link source once; a restored view cannot read again", async () => {
  const route = makePdfRoute(filename, "Outside paper");
  const owner = activatePdfOwnership("/synthetic/graph");
  const read = vi.fn().mockResolvedValue(new TextEncoder().encode("%PDF-1.7\n"));
  authorizeExternalPdf(route.viewId, filename, read);
  const host = document.createElement("div"); document.body.append(host);
  let dispose = render(() => <KeyedPdfViewer route={() => route} owner={() => owner} />, host);
  await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
  dispose();
  dispose = render(() => <KeyedPdfViewer route={() => route} owner={() => owner} />, host);
  try {
    await vi.waitFor(() => expect(host.textContent).toContain("Open the original PDF link again"));
    expect(read).toHaveBeenCalledOnce();
  } finally { dispose(); }
});

it("invalidating a binding drops unused link authorization", () => {
  const read = vi.fn();
  authorizeExternalPdf("view", filename, read);
  invalidateBinding();
  expect(takeExternalPdfSource("view", filename)).toBeUndefined();
  expect(read).not.toHaveBeenCalled();
});

it("explains the read-only tools in Chinese", async () => {
  await loadLocale("zh-CN"); showLocale("zh-CN");
  const { host, dispose } = mount();
  try { expect(host.querySelector(".pdf-read-only")?.textContent).toContain("只读"); }
  finally { dispose(); }
});

it("localizes native refusal tokens and keeps the path in Chinese", async () => {
  await loadLocale("zh-CN"); showLocale("zh-CN");
  const { host, dispose } = mount(() => Promise.reject("external-pdf-not-pdf: file:///tmp/outside%20paper.pdf"));
  try {
    await vi.waitFor(() => expect(host.querySelector(".pdf-load-error")?.textContent).toContain("此文件不是 PDF"));
    expect(host.querySelector(".pdf-load-error")?.textContent).toContain("/tmp/outside paper.pdf");
  } finally { dispose(); }
});

it("shows OS errors with the decoded path once", async () => {
  const { host, dispose } = mount(() => Promise.reject(`Permission denied: ${filename}`));
  try {
    await vi.waitFor(() => expect(host.querySelector(".pdf-load-error")?.textContent).toContain("Permission denied"));
    const message = host.querySelector(".pdf-load-error")!.textContent!;
    expect(message).toContain("/tmp/outside paper.pdf");
    expect(message).not.toContain(filename);
  } finally { dispose(); }
});

// GH #732 follow-up: a relative link reaches the same bounded native read as an
// absolute one, so the native refusals show in the reader with the resolved path.
it.each([
  ["external-pdf-not-regular", "not a regular file"],
  ["external-pdf-too-large", "larger than 256 MiB"],
  ["external-pdf-not-pdf", "not a PDF"],
])("a relative PDF link refused with %s shows its resolved path in the closable reader", async (token, text) => {
  await initParser();
  setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
  const open = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const api = backend(), original = api.readExternalPdf;
  const read = vi.fn((url: string) => Promise.reject(`${token}: ${url}`));
  api.readExternalPdf = read;
  const source = document.createElement("div"); document.body.append(source);
  const disposeSource = render(() => <InlineText text="![paper](file:../my papers/a.pdf)" format="md" />, source);
  source.querySelector<HTMLAnchorElement>("a.pdf-link")!.click();
  const [file, label, , , options] = open.mock.calls[0];
  const owner = activatePdfOwnership("/synthetic/graph");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PdfViewer filename={file} label={label} owner={owner} externalRead={options!.externalRead} />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".pdf-load-error")).not.toBeNull());
    const message = host.querySelector(".pdf-load-error")!.textContent!;
    expect(message).toContain(text);
    expect(message).toContain("/home/user/my papers/a.pdf");
    expect(message).not.toContain("file://");
    expect(host.querySelector(".pdf-close-btn")).not.toBeNull();
    expect(read).toHaveBeenCalledExactlyOnceWith("file:///home/user/my%20papers/a.pdf", api.graphBindingGeneration());
    expect(getDocument).not.toHaveBeenCalled();
  } finally {
    dispose(); disposeSource(); setGraphMeta(null);
    if (original) api.readExternalPdf = original; else delete api.readExternalPdf;
  }
});
