import type { PdfHighlightState } from "./pdfHighlightState";

/** No annotation owner, session, filesystem reads or mutation paths. Even a
 * future caller bypassing hidden controls cannot persist an outside highlight. */
export function readOnlyPdfHighlights(): { state: PdfHighlightState; release: () => void } {
  const noop = () => {};
  const refused = async () => false;
  const state: PdfHighlightState = {
    highlights: () => [], unsaved: () => false, conflict: () => false,
    decisionBusy: () => false, cleanupPending: () => false, graphCurrent: () => false,
    load: noop, loadInitial: () => false, edit: noop, addCrop: () => new Map(),
    editBlocked: () => true, drainBlocked: () => false, persist: refused,
    persistInsideMutation: refused, useDiskVersion: async () => {}, keepMine: async () => {},
    discardMine: async () => {}, retryCleanup: async () => {}, newIntent: () => () => false,
    attachPrepare: () => noop, flush: async () => true, idle: () => true,
    hold: async () => { throw new Error("outside PDF annotations are disabled"); },
  };
  return { state, release: noop };
}
