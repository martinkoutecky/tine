import { visibleBody } from "../render/block";
import { MARKERS } from "../markers";
import type { FieldSpec } from "./config";
import { groupKeysForBlock, isFieldId, isFormulaField, type FieldId } from "./fields";
import { liveFormulaRowNode, readFormulaRowField, type FormulaEvalRow } from "./formulaEval";
import { readField } from "./fields";
import { recordFacets, rowRaw } from "./tableFields";

/** One board column: its group key (null = "(none)"), label and member rows. */
export interface BoardColumn<R extends FormulaEvalRow = FormulaEvalRow> {
  key: string | null;
  label: string;
  rows: R[];
}

const NONE_LABEL = "(none)";

/** The field a board groups by, from its `tine.group-by` token; state when the
 *  token is absent or names no field. O(1). */
export function boardGroupField(token: string | null | undefined): FieldId {
  const raw = token || "state";
  const normalized = raw.startsWith("formula.") ? `formula:${raw.slice("formula.".length)}` : raw;
  return isFieldId(normalized) ? normalized : "state";
}

/** Formula-editor reference name for a non-formula field; formulas cannot
 * reference formula fields. O(token length), no row lookup or formula evaluation. */
export function formulaReferenceName(field: FieldId): string | null {
  if (isFormulaField(field)) return null;
  return field.startsWith("prop:") ? field.slice(5) : field;
}

function enumValuesFor(schema: readonly FieldSpec[], field: FieldId): readonly string[] | null {
  const spec = schema.find((s) => s.field === field);
  return spec && typeof spec.type === "object" && "enum" in spec.type ? spec.type.enum : null;
}

/** The ONE answer to "which columns does a board show, in which order, holding
 *  which rows" for the live board and the static export. Cost O(rows x tags). */
export function buildBoardColumns<R extends FormulaEvalRow>(
  rows: readonly R[],
  groupBy: FieldId,
  schema: readonly FieldSpec[],
  opts: { formulas?: ReadonlyMap<string, string>; now?: Date; workflow: "todo" | "now"; onRow?: (rowId: string) => void }
): BoardColumn<R>[] {
  const rowsByKey = new Map<string | null, R[]>();
  const keys: (string | null)[] = [];
  const allKeys: (string | null)[] = [];
  const seenAllKeys = new Set<string | null>();
  let hasNull = false;
  let hasFormulaError = false;
  for (const row of rows) {
    opts.onRow?.(row.id);
    const rowKeys = groupKeysForBlock(row, groupBy, opts);
    keys.push(rowKeys[0] ?? null);
    const seenForRow = new Set<string | null>();
    for (const key of rowKeys) {
      hasNull ||= key === null;
      hasFormulaError ||= key === "(error)";
      if (!seenAllKeys.has(key)) {
        seenAllKeys.add(key);
        allKeys.push(key);
      }
      if (seenForRow.has(key)) continue;
      seenForRow.add(key);
      const bucket = rowsByKey.get(key);
      if (bucket) bucket.push(row);
      else rowsByKey.set(key, [row]);
    }
  }
  let order: (string | null)[];
  const enumValues = enumValuesFor(schema, groupBy);
  if (isFormulaField(groupBy)) {
    const present = new Set(keys.filter((key): key is string => key !== null));
    const booleanish = present.has("true") || present.has("false");
    order = [];
    if (booleanish) {
      if (present.has("true")) order.push("true");
      if (present.has("false")) order.push("false");
    }
    for (const key of keys) {
      if (key === null || key === "(error)") continue;
      if (booleanish && (key === "true" || key === "false")) continue;
      if (!order.includes(key)) order.push(key);
    }
  } else if (groupBy === "tags") {
    order = [];
    for (const key of allKeys) if (key !== null) order.push(key);
  } else if (enumValues) {
    order = [...enumValues];
    for (const key of keys) if (key !== null && !order.includes(key)) order.push(key);
    order.push(null);
  } else if (groupBy === "state") {
    const standard = opts.workflow === "todo" ? ["TODO", "DOING", "DONE"] : ["LATER", "NOW", "DONE"];
    order = [
      ...standard,
      ...MARKERS.filter((m) => !standard.includes(m) && keys.includes(m)),
    ];
  } else if (groupBy === "priority") {
    order = ["A", "B", "C"];
  } else {
    order = [];
    for (const key of keys) if (key !== null && !order.includes(key)) order.push(key);
  }
  if (hasNull && !order.includes(null)) order.push(null);
  if (isFormulaField(groupBy) && hasFormulaError && !order.includes("(error)")) {
    order.push("(error)");
  }
  if (order.length === 0) order = [null];
  return order.map((key) => ({
    key,
    label: key === null ? NONE_LABEL : groupBy === "priority" ? `[#${key}]` : key,
    rows: rowsByKey.get(key) ?? [],
  }));
}

/** A board card's title: the first visible body line. O(1). */
export function boardRowTitle(row: FormulaEvalRow): string {
  return visibleBody(rowRaw(row))[0] ?? "";
}

/** One field's text as the card chips show it, live or from a DTO. O(1). */
function chipText(row: FormulaEvalRow, field: FieldId): string {
  if (liveFormulaRowNode(row)) return readField(row.id, field)?.text ?? "";
  if (isFormulaField(field)) return "";
  const f = recordFacets(row);
  if (!f) return "";
  if (field === "state") return f.marker ?? "";
  if (field === "priority") return f.priority ?? "";
  if (field === "scheduled") return f.scheduled ?? "";
  if (field === "deadline") return f.deadline ?? "";
  if (field === "tags") return f.tags.join(" ");
  if (field === "page") return row.page;
  return f.properties.find(([k]) => k === field.slice(5))?.[1] ?? "";
}

/** The chips under a card's title; the field the board groups by is never repeated. */
export function boardCardChips(row: FormulaEvalRow, groupBy: FieldId): { priority: string; scheduled: string; deadline: string; tags: string[] } {
  const value = (field: FieldId) => (groupBy === field ? "" : chipText(row, field));
  return {
    priority: value("priority"),
    scheduled: value("scheduled"),
    deadline: value("deadline"),
    // Members come from the field reader, so a multi-word tag stays one chip.
    tags: groupBy === "tags" ? [] : (readFormulaRowField(row, "tags")?.items ?? []).map((t) => (t.startsWith("#") ? t : `#${t}`)),
  };
}
