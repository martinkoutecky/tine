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

/** The face a block's own query draws. One sheet per block (tine#874): a grid draws its
 * children, so its own query draws as the list, never as a second table or board (two
 * sheets of one block share one selection key, so keys meant for one acted on the
 * other). Nothing is written: removing the grid property brings the table or board
 * back. Reads only the grid property and the query's face: O(1), no body scan. */
export function blockSheet<F extends string>(cfg: SheetConfig, queryFace: F): F | "list" {
  return cfg.view === "grid" && (queryFace === "table" || queryFace === "board") ? "list" : queryFace;
}
