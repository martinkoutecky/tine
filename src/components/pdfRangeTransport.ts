import * as pdfjs from "pdfjs-dist";
import type { PdfAssetSource } from "../pdfAssetSource";

/** Match the native IPC allocation ceiling even when pdf.js coalesces ranges. */
export const PDF_RANGE_CHUNK_SIZE = 1_048_576;

export function createPdfRangeTransport(source: PdfAssetSource, current: () => boolean) {
  let active = true;
  let reads = Promise.resolve();
  let failed!: (message: string) => void;
  const failure = new Promise<string>((resolve) => { failed = resolve; });
  class Transport extends pdfjs.PDFDataRangeTransport {
    constructor() { super(source.length, new Uint8Array(0), true); }
    abort() { active = false; }
    requestDataRange(begin: number, end: number) {
      if (!active) return;
      reads = reads.then(async () => {
        if (!active) return;
        if (!Number.isSafeInteger(begin) || !Number.isSafeInteger(end) || begin < 0 || end <= begin || end > source.length)
          throw new Error("Invalid PDF byte range");
        // pdf.js 4's range reader consumes one complete response. Coalesced
        // requests need assembly, but each native allocation/IPC stays capped.
        const combined = end - begin > PDF_RANGE_CHUNK_SIZE ? new Uint8Array(end - begin) : undefined;
        for (let offset = begin; offset < end; offset += PDF_RANGE_CHUNK_SIZE) {
          if (!active || !current()) return;
          const limit = Math.min(end, offset + PDF_RANGE_CHUNK_SIZE);
          const bytes = await source.read(offset, limit);
          if (!active || !current()) return;
          if (bytes.byteLength !== limit - offset) throw new Error("PDF asset changed or a byte range was truncated");
          if (combined) combined.set(bytes, offset - begin);
          else this.onDataRange(begin, bytes);
        }
        if (combined && active) this.onDataRange(begin, combined);
      }).catch((error: unknown) => {
        if (!active) return;
        active = false;
        failed(`Couldn't read this PDF asset: ${String(error)}`);
      });
    }
  }
  return { range: new Transport(), failure };
}
