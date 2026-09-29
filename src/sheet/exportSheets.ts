// The app half of a sheet-carrying static export (family 7): ask the Rust core
// which blocks are sheets, compute each with the app's own sheet code, and hand
// the data back for the publisher to lay out. Every in-app export path (publish,
// query publish, print/PDF) calls this one function, so a path cannot forget it.
import { backend } from "../backend";
import { appNow } from "../journal";
import { graphOwner, readOwned } from "../owned";
import { workflow } from "../ui";
import { computeSheetExports, type SheetExport } from "./staticExport";

/** Sheets of `pages` (every page when omitted), computed against the live app
 * settings. Never rejects: a sheet that cannot be computed is left out (or
 * exported as an error record) and renders as its plain outline. Inputs read
 * for a graph that is no longer current are dropped. */
export async function exportSheets(pages?: string[]): Promise<SheetExport[]> {
  const owner = graphOwner();
  try {
    const read = await readOwned(owner, backend().sheetExportInputs(pages));
    if (read.kind !== "current") return [];
    return computeSheetExports(read.value, workflow(), appNow());
  } catch {
    return [];
  }
}
