import * as pdfjs from "pdfjs-dist";
import type { Backend } from "../backendInterface";
import { errorFamily } from "../errorFamily";
import { bindingOwner, readOwned, type WriteOwner } from "../owned";
import { errorMessage, MAX_PDF_BYTES } from "./pdfViewerPrimitives";
import { createPdfRangeTransport, PDF_RANGE_CHUNK_SIZE } from "./pdfRangeTransport";
import type { OpenPdfDocument } from "./pdfDocumentShare";
import { pdfjsAssetOptions } from "./pdfjsAssets";
import { isExternalPdf, externalPdfPath } from "../render/externalPdfPath";
import { t } from "../i18n";
import type { ExternalPdfRead } from "../externalPdfSource";

const TOO_LARGE = "This PDF is larger than 256 MiB and can't be opened safely.";

/** Native refusal tokens are catalog text; OS errors remain verbatim details. */
function outsideReadError(error: unknown, filename: string): string {
  const raw = error instanceof Error ? error.message : String(error);
  // Native refusals retain their URL for diagnostics; the reader shows the
  // decoded path once, beside the message, instead of repeating the long URL.
  const suffix = `: ${filename}`;
  const detail = raw.endsWith(suffix) ? raw.slice(0, -suffix.length) : raw;
  const token = detail.split(":", 1)[0];
  switch (token) {
    case "external-pdf-not-pdf": return t("pdf-external-not-pdf");
    case "external-pdf-not-regular": return t("pdf-external-not-regular");
    case "external-pdf-too-large": return t("pdf-external-too-large");
    case "external-pdf-invalid-link": return t("pdf-external-invalid-link");
    case "external-pdf-unsupported-url": return t("pdf-external-unsupported-url");
    case "external-pdf-desktop-only": return t("pdf-external-desktop-only");
    default: return detail;
  }
}

/** The shared document owns its transport, including cancellation and failures. */
export function openPdfDocument(api: Backend, filename: string, generation: number,
  current: WriteOwner, externalRead?: ExternalPdfRead): OpenPdfDocument {
  const loadOwner = bindingOwner(current);
  const outside = externalPdfPath(filename);
  const errorPath = outside.kind === "pdf" ? outside.path : filename;
  return async (adopt) => {
    let input: Parameters<typeof pdfjs.getDocument>[0];
    let transport: ReturnType<typeof createPdfRangeTransport> | undefined;
    try {
      if (isExternalPdf(filename)) {
        if (!externalRead) throw new Error(t("pdf-external-reopen-link"));
        const result = await readOwned(loadOwner, externalRead());
        if (result.kind === "stale") return { kind: "error", message: t("pdf-external-reopen-link") };
        if (result.value.byteLength > MAX_PDF_BYTES) throw new Error("external-pdf-too-large");
        input = { data: result.value };
      } else if (api.pdfAssetSource) {
        const result = await readOwned(loadOwner, api.pdfAssetSource(filename, MAX_PDF_BYTES, generation));
        if (result.kind === "stale") return { kind: "error", message: "This graph was closed while the PDF was loading." };
        const source = result.value;
        if (source.length > MAX_PDF_BYTES) return { kind: "error", message: TOO_LARGE };
        if (!Number.isSafeInteger(source.length) || source.length <= 0) throw new Error("file is empty or has an invalid length");
        transport = createPdfRangeTransport(source, loadOwner);
        // pdf.js needs both flags to avoid fetching the rest in the background.
        input = { range: transport.range, rangeChunkSize: PDF_RANGE_CHUNK_SIZE, disableStream: true, disableAutoFetch: true };
      } else {
        const result = await readOwned(loadOwner, api.readAsset(filename, MAX_PDF_BYTES));
        if (result.kind === "stale") return { kind: "error", message: "This graph was closed while the PDF was loading." };
        const bytes = result.value;
        if (bytes.byteLength > MAX_PDF_BYTES) return { kind: "error", message: TOO_LARGE };
        if (!bytes.length) throw new Error("file is empty");
        input = { data: bytes };
      }
    } catch (error) {
      if (isExternalPdf(filename)) return { kind: "error", message: `${errorPath}: ${outsideReadError(error, filename)}` };
      return { kind: "error", message: errorFamily(error) === "asset-too-large" ? TOO_LARGE : errorMessage("Couldn't read this PDF asset", error) };
    }
    const task = pdfjs.getDocument({ ...input, ...pdfjsAssetOptions() });
    adopt.task({ destroy: () => { transport?.range.abort(); return task.destroy(); } });
    // Destruction alone need not reject a loading task. Race the explicit read
    // failure too, and let every retaining view observe later range failures.
    const failed = transport?.failure.then((message) => { throw new Error(message); });
    try {
      const doc = await (failed ? Promise.race([task.promise, failed]) : task.promise);
      adopt.document(doc);
      return { kind: "ok", doc, failure: transport?.failure };
    } catch (error) {
      return { kind: "error", message: isExternalPdf(filename)
        ? `${errorPath}: ${t("pdf-external-load-failed", { detail: String(error) })}`
        : errorMessage("Couldn't load this PDF", error) };
    }
  };
}
