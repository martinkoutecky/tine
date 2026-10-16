import type { RefGroup } from "./types";
import type { FormulaEvalRow } from "./sheet/formulaEval";
import { fieldIdsForRecords } from "./sheet/tableFields";
import { queryColumnName } from "./sheet/tablePresentation";

/** Available query columns come only from rows already in the answer. */
export function queryRecordColumns(rows: readonly FormulaEvalRow[]): string[] {
  return fieldIdsForRecords(rows, true).map(queryColumnName)
    .filter((field): field is string => field !== null);
}

export function queryBlockColumns(groups: readonly RefGroup[]): string[] {
  return queryRecordColumns(groups.flatMap(group => group.blocks.map(dto => ({
    id: dto.id, page: group.page, kind: group.kind, path: group.path, dto,
  }))));
}
