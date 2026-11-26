import { queryMacroExtents } from "../editor/queryMacro";
import type { SheetConfig } from "./config";

/** Which sheet a block draws. ONE answer, shared by the live Block (its children), the
 * block's own query macro, a nested grid cell's block and the static export (I-12).
 *
 * One sheet per block (tine#874): a block draws at most one sheet.
 * - A grid draws its children. Its own query then draws as the list, never as a
 *   second sheet: two sheets of one block share one selection key, so keys meant
 *   for one would act on the other. Nothing is written; removing the grid property
 *   brings the query's table or board back.
 * - Otherwise a table/board view on a block whose body CONTAINS a {{query}} macro
 *   belongs to the query results (the macro path draws it): the children face would
 *   draw a SECOND, empty sheet. The macro need not be the whole body: the §4 demo
 *   block is a heading + {{query}} + tine.view:: board in ONE block.
 *
 * `children` is the children-source sheet (`view: null` = the plain outline).
 * `query` is the face the block's own query draws: `queryFace` itself, or "list" in
 * place of a table or board the block cannot draw. Undefined without a `queryFace`.
 * `raw` is read only when `children` of a table/board view needs the query-body check. */
export function blockSheet<F extends string>(cfg: SheetConfig, raw: () => string, queryFace?: F): {
  children: SheetConfig;
  query: F | "list" | undefined;
} {
  const grid = cfg.view === "grid";
  return {
    // A getter: the query macro reads only `query`, so it never pays the body check.
    get children() {
      if (!grid && (cfg.view === "table" || cfg.view === "board") && queryMacroExtents(raw()).length > 0) return { ...cfg, view: null };
      return cfg;
    },
    query: grid && (queryFace === "table" || queryFace === "board") ? "list" : queryFace,
  };
}
