import { backend } from "./backend";
import { pdfFileForPage, pdfFileFromPreBlock } from "./editor/annotation";
import { msg } from "./i18n";
import { graphOwner, readOwned } from "./owned";
import { openPdf } from "./panes";
import { pushToast } from "./toasts";
import type { PageKind } from "./types";

/** An annotation can be visible in a preview before its source page is loaded.
 * Read only its header on click; graph ownership fences the asynchronous reply. */
export function openAnnotationPdf(page: string, hlPage: number, highlightId: string, kind: PageKind = "page"): void {
  const loadedFile = pdfFileForPage(page);
  if (loadedFile) { openPdf(loadedFile, loadedFile, hlPage, highlightId); return; }
  void readOwned(graphOwner(), backend().getPage(page, kind)).then(result => {
    if (result.kind === "stale") return;
    const file = pdfFileFromPreBlock(result.value?.pre_block, result.value?.format);
    if (file) openPdf(file, file, hlPage, highlightId);
    else pushToast(msg("render-couldn-t-find-the-pdf-for-this-highlight"), "error");
  }).catch(() => { pushToast(msg("render-couldn-t-open-the-pdf-for-this-highlight"), "error"); });
}
