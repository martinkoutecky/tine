import type { FieldType } from "./config";
import { isFormulaField, type FieldId, type FieldValue } from "./fields";
import { formulaValueText } from "./formulaEval";
import type { FormulaValue } from "./formula";
import { parseIsoDateLike } from "./typed";

/** How one sheet cell value is presented. The live cell renderer and the static
 *  export both read THIS answer; neither re-decides what a value looks like. */
export type CellView =
  | { k: "none" }
  | { k: "marker"; raw: string; text: string }
  | { k: "priority"; raw: string; text: string }
  | { k: "date"; cls: "scheduled" | "deadline"; text: string }
  | { k: "chips"; values: string[] }
  | { k: "check"; checked: boolean }
  | { k: "inline"; text: string }
  | { k: "error"; message: string }
  | { k: "plain"; text: string };

/** True for a declared enumerated field type. */
export function isEnumFieldType(type: FieldType | undefined): type is { enum: readonly string[] } {
  return typeof type === "object" && type !== null && "enum" in type;
}

/** The value a cell shows: an unset checkbox property reads as `false`. */
export function displayFieldValue(field: FieldId, type: FieldType | undefined, value: FieldValue | null): FieldValue | null {
  if (value) return value;
  return field.startsWith("prop:") && type === "checkbox" ? { text: "false", raw: "false" } : null;
}

function formulaCellView(value: FormulaValue | null): CellView {
  switch (value?.kind) {
    case "error": return { k: "error", message: value.message };
    case "number": case "text": case "duration": return { k: "plain", text: formulaValueText(value) };
    case "date": return { k: "date", cls: "scheduled", text: formulaValueText(value) };
    case "boolean": return { k: "check", checked: value.value };
    case "list": return { k: "chips", values: value.values.map((v) => formulaValueText(v)) };
    default: return { k: "none" };
  }
}

function propCellView(type: FieldType | undefined, value: FieldValue): CellView {
  const raw = (value.raw ?? value.text).trim();
  if (type === "checkbox") {
    const lower = raw.toLowerCase();
    if (lower === "true" || lower === "false") return { k: "check", checked: lower === "true" };
  }
  if ((type === "date" || type === "datetime") && parseIsoDateLike(raw) !== null) return { k: "date", cls: "scheduled", text: raw };
  if (isEnumFieldType(type) && type.enum.includes(raw)) return { k: "chips", values: [raw] };
  if (type === "list") {
    const values = raw.split(",").map((v) => v.trim()).filter(Boolean);
    if (values.length > 0) return { k: "chips", values };
  }
  if (type === "ref" && /^\[\[[^\]\n\r]+\]\]$/.test(raw)) return { k: "inline", text: raw };
  return { k: "inline", text: value.text };
}

/** Presentation of one cell; O(value length), pure. */
export function cellView(field: FieldId, type: FieldType | undefined, value: FieldValue | null, formulaValue?: FormulaValue | null): CellView {
  if (isFormulaField(field)) return formulaCellView(formulaValue ?? null);
  if (!value) return { k: "none" };
  const text = value.text ?? "";
  switch (field) {
    case "state": return { k: "marker", raw: value.raw ?? "", text };
    case "priority": return { k: "priority", raw: value.raw ?? "", text };
    case "scheduled": return { k: "date", cls: "scheduled", text };
    case "deadline": return { k: "date", cls: "deadline", text };
    case "tags": return { k: "chips", values: (value.raw ?? "").split(/\s+/).filter(Boolean).map((t) => `#${t}`) };
    case "page": return { k: "inline", text };
    default: return field.startsWith("prop:") ? propCellView(type, value) : { k: "none" };
  }
}
