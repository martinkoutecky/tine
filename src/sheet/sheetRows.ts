import { node as docNode } from "../document";
import type { RefGroup } from "../types";
import type { FormulaEvalRow } from "./formulaEval";

/** The rows a sheet shows before its filter: an owner block's children, or the
 *  blocks of a query's result groups (carrying their DTO for off-document
 *  rows). The one answerer for table and board. O(rows). */
export function sheetSourceRows(
  rowSource: "children" | "query",
  ownerId: string,
  groups: readonly RefGroup[] | undefined,
): FormulaEvalRow[] {
  if (rowSource === "children") {
    const owner = docNode(ownerId);
    return (owner?.children ?? []).map((id) => ({ id, page: docNode(id)?.page ?? owner?.page ?? "" }));
  }
  return (groups ?? []).flatMap((g) => g.blocks.map((b) => ({ id: b.id, page: g.page, kind: g.kind, dto: b })));
}
