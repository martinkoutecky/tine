import { facetsOf } from "../render/facets";
import type { Format } from "../render/ast";
import { isAggregateFn, type AggregateFn } from "./aggregate";
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
const BUILTIN_FIELDS = new Set<FieldId>(["state", "priority", "scheduled", "deadline", "tags", "page"]);
const PROP_FIELD_TYPES = new Set<FieldType>(["text", "number", "date", "datetime", "checkbox", "list", "ref"]);

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
    const m = /^\s*([^=;\s][^=;]*)\s*=\s*([a-z-]+)\s*$/.exec(part);
    if (!m) continue;
    const key = m[1].trim();
    const fn = m[2].toLowerCase();
    if (key && isAggregateFn(fn)) out.set(key, fn);
  }
  return out;
}

export function parseFields(value: string): readonly FieldSpec[] {
  const out: FieldSpec[] = [];
  const seen = new Set<string>();
  for (const part of value.split(";")) {
    const eq = part.indexOf("=");
    if (eq < 0) continue;
    const name = part.slice(0, eq).trim();
    const token = part.slice(eq + 1).trim();
    if (!scalarSafe(name) || seen.has(name)) continue;

    if (BUILTIN_FIELDS.has(name as FieldId)) {
      if (token !== name) continue;
      seen.add(name);
      out.push({ field: name as FieldId, type: "builtin" });
      continue;
    }

    let type: FieldType | null = null;
    if (PROP_FIELD_TYPES.has(token as FieldType)) {
      type = token as FieldType;
    } else if (token.startsWith("enum:")) {
      const values = token
        .slice("enum:".length)
        .split(",")
        .map((v) => v.trim())
        .filter(Boolean);
      if (values.length > 0 && values.every(scalarSafe)) type = { enum: values };
    }
    if (!type) continue;
    seen.add(name);
    out.push({ field: `prop:${name}`, type });
  }
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
    if (BUILTIN_FIELDS.has(spec.field)) {
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
      if (spec.type !== "builtin" && PROP_FIELD_TYPES.has(spec.type)) token = spec.type;
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
