import type { Backend } from "../backendInterface";
import { msg } from "../i18n";
import { graphOwner, readOwned, type Owner } from "../owned";
import type { PdfState } from "../types";
import { pushToast } from "../toasts";

/** Current annotation failures toast while the document remains readable.
 * Outside PDFs bypass the graph annotation reader entirely. */
export async function loadPdfAnnotations(api: Backend, filename: string, label: string,
  generation: number, owner: Owner, readOnly: boolean): Promise<{ kind: "ok"; state: PdfState } | { kind: "unavailable" }> {
  if (readOnly) return { kind: "unavailable" };
  const current = graphOwner(owner);
  try {
    const result = await readOwned(current, api.openPdf(filename, label, generation));
    return result.kind === "stale" ? { kind: "unavailable" } : { kind: "ok", state: result.value };
  } catch (error) {
    if (owner()) pushToast(msg("pdf-couldn-t-load-pdf-annotations", { detail: String(error) }), "error");
    return { kind: "unavailable" };
  }
}
