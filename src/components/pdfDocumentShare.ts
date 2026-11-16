import type * as pdfjs from "pdfjs-dist";
import { discardPdfDocument } from "./pdfViewerPrimitives";

/** What a view gets from the shared load: the document, or the message its
 * error surface shows. */
export type SharedPdfDocument =
  | { kind: "ok"; doc: pdfjs.PDFDocumentProxy; failure?: Promise<string> }
  | { kind: "error"; message: string };

/** Starts one load: read the bytes, hand the pdf.js loading task over as soon
 * as it exists (so a last release can cancel it), hand the document over in
 * the same microtask its task resolves (from then on it owns destruction), and
 * resolve with the outcome. */
export type OpenPdfDocument = (adopt: {
  task(task: { destroy(): Promise<void> }): void;
  document(doc: pdfjs.PDFDocumentProxy): void;
}) => Promise<SharedPdfDocument>;

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
        result = await open({
          task: (task) => {
            if (!created.released) { created.task = task; return; }
            created.cancelled = true; // every view left while the bytes were read
            discardPdfDocument(task);
          },
          document: (doc) => {
            created.task = null;
            if (!created.released) created.doc = doc;
            else if (!created.cancelled) discardPdfDocument(doc);
          },
        });
      } catch (error) {
        result = { kind: "error", message: String(error) };
      }
      const task = created.task;
      created.task = null;
      if (result.kind === "error") {
        // pdf.js does not dispose a task whose parse rejected: its worker and
        // transport stay alive until the task is destroyed.
        if (task && !created.cancelled) { created.cancelled = true; discardPdfDocument(task); }
        if (entries.get(key) === created) entries.delete(key);
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
