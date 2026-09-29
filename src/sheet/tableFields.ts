import { formatForBlock } from "../document";
import { facetsFromDto, facetsOf, type Facets } from "../render/facets";
import { isRenderHiddenProp } from "../render/block";
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

