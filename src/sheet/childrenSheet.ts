import { queryMacroExtents } from "../editor/queryMacro";
import { sheetConfig, type SheetConfig } from "./config";

/** The children-source sheet a block owns, or `view: null`. A table/board view on a
 * block whose body CONTAINS a {{query}} macro belongs to the query results (the macro
 * path renders it, rowSource: query) — the children-source face would render a SECOND,
 * empty sheet below it. The macro need not be the whole body: the §4 demo block is a
 * heading + {{query}} + tine.view:: board in ONE block. Grid stays children-source even
 * on a query block. ONE answer, shared by the live Block and the static export (I-12). */
export function childrenSheetConfig(properties: readonly [string, string][], raw: string): SheetConfig {
  const cfg = sheetConfig(properties);
  if ((cfg.view === "table" || cfg.view === "board") && queryMacroExtents(raw).length > 0) return { ...cfg, view: null };
  return cfg;
}
