import { formatForBlock } from "../document";
import { facetsFromDto, facetsOf, type Facets } from "../render/facets";
import { isRenderHiddenProp, visibleBody } from "../render/block";
import { liveFormulaRowNode, type FormulaEvalRow } from "./formulaEval";
import type { FieldId } from "./fields";

/** Facets for one live or DTO query row; cost is O(one block). */
export function recordFacets(row: FormulaEvalRow): Facets | null {
  const n = liveFormulaRowNode(row);
  if (n) return facetsOf(n.raw, formatForBlock(row.id));
  return row.dto ? facetsFromDto(row.dto) : null;
}

/** Union observed fields over the supplied rows; cost is O(rows and their properties). */
export function fieldIdsForRecords(rows: readonly FormulaEvalRow[], includePage: boolean): FieldId[] {
  const out: FieldId[] = [];
  const props: FieldId[] = [];
  const seenProps = new Set<string>();
  let hasState = false;
  let hasPriority = false;
  let hasScheduled = false;
  let hasDeadline = false;
  let hasTags = false;
  for (const r of rows) {
    const f = recordFacets(r);
    if (!f) continue;
    hasState ||= !!f.marker;
    hasPriority ||= !!f.priority;
    hasScheduled ||= !!f.scheduled;
    hasDeadline ||= !!f.deadline;
    hasTags ||= f.tags.length > 0;
    for (const [key] of f.properties) {
      if (isRenderHiddenProp(key)) continue;
      const field: FieldId = `prop:${key}`;
      if (!seenProps.has(field)) {
        seenProps.add(field);
        props.push(field);
      }
    }
  }
  if (hasState) out.push("state");
  if (hasPriority) out.push("priority");
  if (hasScheduled) out.push("scheduled");
  if (hasDeadline) out.push("deadline");
  if (hasTags) out.push("tags");
  out.push(...props);
  if (includePage) out.push("page");
  return out;
}


/** Table columns after the title: declared fields, then formulas, then inferred
 *  (observed plus user-added) ones not already named. O(fields). */
export function tableFieldOrder(
  observed: readonly FieldId[],
  extra: readonly FieldId[],
  declared: readonly FieldId[],
  formulas: readonly FieldId[]
): FieldId[] {
  const seen = new Set(observed);
  const inferred = [...observed, ...extra.filter((f) => !seen.has(f))];
  const named = new Set<FieldId>([...declared, ...formulas]);
  return [...declared, ...formulas, ...inferred.filter((f) => !named.has(f))];
}

/** A row's raw text, live or from its DTO. O(1). */
export function rowRaw(row: FormulaEvalRow): string {
  return liveFormulaRowNode(row)?.raw ?? row.dto?.raw ?? "";
}

/** A table row's title cell text; an empty title over children reads as an em dash. */
export function tableRowTitle(row: FormulaEvalRow): string {
  const title = visibleBody(rowRaw(row)).join(" ");
  return title.trim() === "" && (liveFormulaRowNode(row)?.children.length ?? row.dto?.children.length ?? 0) > 0 ? "—" : title;
}
