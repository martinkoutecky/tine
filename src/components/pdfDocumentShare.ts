import type * as pdfjs from "pdfjs-dist";
import { discardPdfDocument } from "./pdfViewerPrimitives";

/** What a view gets from the shared load: the document, or the message its
 * error surface shows. */
export type SharedPdfDocument =
  | { kind: "ok"; doc: pdfjs.PDFDocumentProxy }
  | { kind: "error"; message: string };

/** Starts one load: read the bytes, hand the pdf.js loading task over as soon
 * as it exists (so a last release can cancel it), resolve with the outcome. */
export type OpenPdfDocument = (adoptTask: (task: { destroy(): Promise<void> }) => void) => Promise<SharedPdfDocument>;

interface Entry {
  loaded: Promise<SharedPdfDocument>;
  views: number;
  task: { destroy(): Promise<void> } | null;
  doc: pdfjs.PDFDocumentProxy | null;
  released: boolean;
  /** The loading task was destroyed, which also retires the document it yields. */
  cancelled: boolean;
}

const entries = new Map<string, Entry>();

/** Share one pdf.js document (bytes, worker, page proxies) among every view of
 * a PDF in one graph binding (PDF tabs and splits design, 5.1): another view
 * of an open PDF costs no second copy. The first view's `open` loads it; a
 * failed load is forgotten so the next view retries. The last release
 * destroys the document, or cancels the load still under way. A file changed
 * on disk is read again once every view of it has closed. Cost O(1) per
 * acquire/release beyond the one load. */
export function acquirePdfDocument(key: string, open: OpenPdfDocument): {
  loaded: Promise<SharedPdfDocument>;
  release: () => void;
} {
  let entry = entries.get(key);
  if (!entry) {
    const created: Entry = { loaded: null!, views: 0, task: null, doc: null, released: false, cancelled: false };
    created.loaded = (async (): Promise<SharedPdfDocument> => {
      let result: SharedPdfDocument;
      try {
        result = await open((task) => {
          if (!created.released) { created.task = task; return; }
          created.cancelled = true; // every view left while the bytes were read
          discardPdfDocument(task);
        });
      } catch (error) {
        result = { kind: "error", message: String(error) };
      }
      created.task = null;
      if (result.kind === "ok") {
        if (created.released) { if (!created.cancelled) discardPdfDocument(result.doc); }
        else created.doc = result.doc;
      } else if (entries.get(key) === created) {
        entries.delete(key);
      }
      return result;
    })();
    entries.set(key, created);
    entry = created;
  }
  const held = entry;
  held.views++;
  let released = false;
  return {
    loaded: held.loaded,
    release: () => {
      if (released) return;
      released = true;
      if (--held.views > 0) return;
      held.released = true;
      if (entries.get(key) === held) entries.delete(key);
      if (held.doc) discardPdfDocument(held.doc);
      else if (held.task) { held.cancelled = true; discardPdfDocument(held.task); }
      held.doc = null;
      held.task = null;
    },
  };
}

/** Live shared documents (test and probe use). */
export function sharedPdfDocumentCount(): number {
  return entries.size;
}
