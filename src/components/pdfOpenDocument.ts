import * as pdfjs from "pdfjs-dist";
import type { Backend } from "../backendInterface";
import { errorFamily } from "../errorFamily";
import { bindingOwner, readOwned, type WriteOwner } from "../owned";
import { errorMessage, MAX_PDF_BYTES } from "./pdfViewerPrimitives";
import { createPdfRangeTransport, PDF_RANGE_CHUNK_SIZE } from "./pdfRangeTransport";
import type { OpenPdfDocument } from "./pdfDocumentShare";

const TOO_LARGE = "This PDF is larger than 256 MiB and can't be opened safely.";

/** The shared document owns its transport, including cancellation and failures. */
export function openPdfDocument(api: Backend, filename: string, generation: number,
  current: WriteOwner): OpenPdfDocument {
  const loadOwner = bindingOwner(current);
  return async (adopt) => {
    let input: Parameters<typeof pdfjs.getDocument>[0];
    let transport: ReturnType<typeof createPdfRangeTransport> | undefined;
    try {
      if (api.pdfAssetSource) {
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
      return { kind: "error", message: errorFamily(error) === "asset-too-large" ? TOO_LARGE : errorMessage("Couldn't read this PDF asset", error) };
    }
    const task = pdfjs.getDocument(input);
    adopt.task({ destroy: () => { transport?.range.abort(); return task.destroy(); } });
    // Destruction alone need not reject a loading task. Race the explicit read
    // failure too, and let every retaining view observe later range failures.
    const failed = transport?.failure.then((message) => { throw new Error(message); });
    try {
      const doc = await (failed ? Promise.race([task.promise, failed]) : task.promise);
      adopt.document(doc);
      return { kind: "ok", doc, failure: transport?.failure };
    } catch (error) {
      return { kind: "error", message: errorMessage("Couldn't load this PDF", error) };
    }
  };
}
