import { facetsOf } from "../render/facets";
import type { Format } from "../render/ast";
import { decodeAggregateSegment, isAggregateFn, type AggregateFn } from "./aggregate";
import type { FieldId } from "./fields";
import { decodeFormulaExpr } from "./formula";

export type SheetView = "table" | "grid" | "board";
export type FieldType =
  | "text"
  | "number"
  | "date"
  | "datetime"
  | "checkbox"
  | "list"
  | "ref"
  | { enum: readonly string[] }
  | "builtin";
export interface FieldSpec {
  field: FieldId;
  type: FieldType;
}

export interface SheetConfig {
  view: SheetView | null;
  groupBy: string | null;
  header: boolean;
  colWidths: ReadonlyMap<number, number>;
  colAggregates: ReadonlyMap<string, AggregateFn>;
  fields: readonly FieldSpec[];
  filter: string | null;
}

const VIEWS = new Set<SheetView>(["table", "grid", "board"]);
export const TABLE_COLUMN_MAX_WIDTH = 1600;
const TABLE_COLUMN_MIN_WIDTH = 64;
const TABLE_COLUMN_WIDTH_LIMIT = 256;
const TABLE_COLUMN_KEY_LIMIT = 512;
const tableColumnKeyValid = (key: string) => key.length > 0 && key.length <= TABLE_COLUMN_KEY_LIMIT && !/[\0-\x1f\x7f]/.test(key);

/** Parse identity-keyed table widths from `tine.table-widths`; invalid entries
 * are ignored and work is bounded to 256 accepted columns. */
export function parseTableColumnWidths(value: string): ReadonlyMap<string, number> {
  const out = new Map<string, number>();
  for (const part of value.split(";")) {
    const eq = part.indexOf("=");
    if (eq <= 0 || part.indexOf("=", eq + 1) >= 0) continue;
    const token = part.slice(eq + 1).trim();
    if (!/^\d+$/.test(token)) continue;
    const width = Number(token);
    if (!Number.isSafeInteger(width) || width < TABLE_COLUMN_MIN_WIDTH || width > TABLE_COLUMN_MAX_WIDTH) continue;
    try {
      const key = decodeURIComponent(part.slice(0, eq).trim());
      if (!tableColumnKeyValid(key)) continue;
      out.set(key, width);
      if (out.size >= TABLE_COLUMN_WIDTH_LIMIT) break;
    } catch { /* malformed URI escapes are isolated to one entry */ }
  }
  return out;
}

/** Serialize valid identity-keyed table widths deterministically. Work is
 * bounded to 256 columns; callers publish through the document edit door. */
export function serializeTableColumnWidths(widths: ReadonlyMap<string, number>): string {
  return [...widths.entries()]
    .filter(([key, width]) => tableColumnKeyValid(key) && Number.isFinite(width))
    .map(([key, width]) => [encodeURIComponent(key), Math.min(TABLE_COLUMN_MAX_WIDTH, Math.max(TABLE_COLUMN_MIN_WIDTH, Math.round(width)))] as const)
    .sort(([left], [right]) => left < right ? -1 : left > right ? 1 : 0)
    .slice(0, TABLE_COLUMN_WIDTH_LIMIT)
    .map(([key, width]) => `${key}=${width}`).join(";");
}
export const SHEET_BUILTIN_FIELDS = ["state", "priority", "scheduled", "deadline", "tags", "page"] as const;
export const SCHEMA_PROP_TYPES = ["text", "number", "date", "datetime", "checkbox", "list", "ref"] as const;
function isSchemaPropType(value: string): value is typeof SCHEMA_PROP_TYPES[number] {
  return (SCHEMA_PROP_TYPES as readonly string[]).includes(value);
}

export function isSheetBuiltinField(name: string): boolean {
  return (SHEET_BUILTIN_FIELDS as readonly string[]).includes(name);
}

/** Visit stored schema entries, including unsupported types, O(value bytes).
 * An already split value avoids resplitting for lossless writers.
 * Key offsets are UTF-16 within each segment; callbacks can preserve every byte
 * during rename. No entry objects are allocated. Type admission is parseFields'
 * policy; malformed/unknown entries remain available to a lossless writer. */
export function visitFieldSchema(value: string | readonly string[], visit: (segment: string, index: number, name: string, keyStart: number, valueStart: number) => void): void {
  const segments = typeof value === "string" ? value.split(";") : value;
  for (let index = 0; index < segments.length; index++) {
    const segment = segments[index];
    const eq = segment.indexOf("=");
    if (eq < 0) continue;
    const left = segment.slice(0, eq);
    const name = left.trim();
    visit(segment, index, name, left.indexOf(name), eq + 1);
  }
}

function scalarSafe(value: string): boolean {
  return value.trim() !== "" && !/(\[\[|\(\(|\{\{|#|`|[=;\n\r])/.test(value);
}

function parseColWidths(value: string): ReadonlyMap<number, number> {
  const out = new Map<number, number>();
  for (const part of value.split(";")) {
    const m = /^\s*(\d+)\s*=\s*(\d+)\s*$/.exec(part);
    if (!m) continue;
    out.set(Number(m[1]), Number(m[2]));
  }
  return out;
}

function parseColAggregates(value: string): ReadonlyMap<string, AggregateFn> {
  const out = new Map<string, AggregateFn>();
  for (const part of value.split(";")) {
    const segment = decodeAggregateSegment(part, "sheet");
    if (segment) out.set(part.slice(segment.keyStart, segment.keyEnd), segment.fn as AggregateFn);
  }
  return out;
}

export function parseFields(value: string): readonly FieldSpec[] {
  const out: FieldSpec[] = [];
  const seen = new Set<string>();
  visitFieldSchema(value, (segment, _index, name, _keyStart, valueStart) => {
    if (!scalarSafe(name) || seen.has(name)) return;
    const token = segment.slice(valueStart).trim();

    if (isSheetBuiltinField(name)) {
      if (token !== name) return;
      seen.add(name);
      out.push({ field: name as FieldId, type: "builtin" });
      return;
    }

    let type: FieldType | null = null;
    if (isSchemaPropType(token)) {
      type = token as FieldType;
    } else if (token.startsWith("enum:")) {
      const values = token
        .slice("enum:".length)
        .split(",")
        .map((v) => v.trim())
        .filter(Boolean);
      if (values.length > 0 && values.every(scalarSafe)) type = { enum: values };
    }
    if (!type) return;
    seen.add(name);
    out.push({ field: `prop:${name}`, type });
  });
  return out;
}

export function serializeColWidths(widths: ReadonlyMap<number, number>): string {
  return [...widths.entries()]
    .filter(([col, px]) => Number.isInteger(col) && col >= 0 && Number.isFinite(px) && px >= 0)
    .sort(([a], [b]) => a - b)
    .map(([col, px]) => `${col}=${Math.round(px)}`)
    .join(";");
}

export function serializeColAggregates(aggregates: ReadonlyMap<string, AggregateFn>): string {
  return [...aggregates.entries()]
    .filter(([key, fn]) => key.trim() && !/[=;\n\r]/.test(key) && isAggregateFn(fn))
    .sort(([a], [b]) => {
      const ai = /^\d+$/.test(a) ? Number(a) : null;
      const bi = /^\d+$/.test(b) ? Number(b) : null;
      if (ai != null && bi != null) return ai - bi;
      if (ai != null) return -1;
      if (bi != null) return 1;
      return a.localeCompare(b);
    })
    .map(([key, fn]) => `${key}=${fn}`)
    .join(";");
}

export function serializeFields(fields: readonly FieldSpec[]): string {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const spec of fields) {
    if (isSheetBuiltinField(spec.field)) {
      if (spec.type !== "builtin" || seen.has(spec.field)) continue;
      seen.add(spec.field);
      out.push(`${spec.field}=${spec.field}`);
      continue;
    }
    if (!spec.field.startsWith("prop:")) continue;
    const name = spec.field.slice(5).trim();
    if (!scalarSafe(name) || seen.has(name)) continue;
    let token: string | null = null;
    if (typeof spec.type === "string") {
      if (spec.type !== "builtin" && isSchemaPropType(spec.type)) token = spec.type;
    } else {
      const values = spec.type.enum.map((v) => v.trim()).filter(Boolean);
      if (values.length > 0 && values.every(scalarSafe)) token = `enum:${values.join(",")}`;
    }
    if (!token) continue;
    seen.add(name);
    out.push(`${name}=${token}`);
  }
  return out.join(";");
}

export function sheetConfig(props: readonly [string, string][]): SheetConfig {
  let view: SheetView | null = null;
  let groupBy: string | null = null;
  let header = false;
  let colWidths: ReadonlyMap<number, number> = new Map();
  let colAggregates: ReadonlyMap<string, AggregateFn> = new Map();
  let fields: readonly FieldSpec[] = [];
  let filter: string | null = null;

  for (const [rawKey, rawValue] of props) {
    const key = rawKey.trim().toLowerCase();
    const value = rawValue.trim();
    if (key === "tine.view") {
      const lower = value.toLowerCase();
      view = VIEWS.has(lower as SheetView) ? (lower as SheetView) : null;
    } else if (key === "tine.group-by") {
      groupBy = value || null;
    } else if (key === "tine.header") {
      header = value.toLowerCase() === "true";
    } else if (key === "tine.col-widths") {
      colWidths = parseColWidths(value);
    } else if (key === "tine.col-aggregates") {
      colAggregates = parseColAggregates(value);
    } else if (key === "tine.fields") {
      fields = parseFields(value);
    } else if (key === "tine.filter") {
      filter = value ? decodeFormulaExpr(value) : null;
    }
  }

  return { view, groupBy, header, colWidths, colAggregates, fields, filter };
}

/** Sheet config straight from a block's raw text, through the ONE block-property
 *  recognizer (`facetsOf`, lsdoc-backed + memoized) — never a second `key::` /
 *  drawer line scanner here (a duplicate recognizer drifts: fence-awareness,
 *  org drawer edge cases). */
export function sheetConfigFromRaw(raw: string, format: Format): SheetConfig {
  return sheetConfig(facetsOf(raw, format).properties);
}
