import { createMemo, type Accessor } from "solid-js";
import { dataRev } from "../ui";
import { doc, formatForBlock, pageByName, type Node as StoreNode } from "../store";
import { facetsFromDto, facetsOf } from "../render/facets";
import { evaluate, parseFormula, type Ast, type FormulaValue, type ParseResult } from "./formula";
import {
  errorValue,
  nullValue,
  numberValue,
  textValue,
  booleanValue,
  parseDateValue,
} from "./formula/value";
import { isPlainDecimalNumber, isoDatePrefix } from "./typed";
import { readField, type FieldId, type FieldValue } from "./fields";
import type { BlockDto, PageKind } from "../types";

export interface FormulaEvalRow {
  id: string;
  page: string;
  kind?: PageKind;
  dto?: BlockDto;
}

export function formulaRowKey(row: Pick<FormulaEvalRow, "id" | "page" | "kind">): string {
  return row.kind ? `${row.kind}\0${row.page}\0${row.id}` : row.id;
}

export function liveFormulaRowNode(row: FormulaEvalRow): StoreNode | null {
  const node = doc.byId[row.id];
  if (!node || node.page !== row.page) return null;
  if (row.kind && pageByName(row.page)?.kind !== row.kind) return null;
  return node;
}

interface FormulaResultsOptions {
  rows: Accessor<readonly FormulaEvalRow[]>;
  formulas: Accessor<ReadonlyMap<string, string>>;
  now?: Accessor<Date>;
  ownerId?: string;
  warnThreshold?: number;
  onEvaluate?: () => void;
}

interface FormulaFilterOptions<T extends FormulaEvalRow> {
  rows: Accessor<readonly T[]>;
  formulas: Accessor<ReadonlyMap<string, string>>;
  filter: Accessor<string | null>;
  now?: Accessor<Date>;
  ownerId?: string;
}

export interface FormulaFilterState<T extends FormulaEvalRow = FormulaEvalRow> {
  rows: readonly T[];
  error: string | null;
}

const BUILTIN_FIELD_NAMES = new Set(["state", "priority", "scheduled", "deadline", "tags", "page"]);
const DEFAULT_EVAL_WARN_THRESHOLD = 10_000;
const parseCache = new Map<string, ParseResult>();

function parseCached(expr: string): ParseResult {
  const cached = parseCache.get(expr);
  if (cached) return cached;
  if (parseCache.size > 500) parseCache.clear(); // editor iterations must not grow this unbounded
  const parsed = parseFormula(expr);
  parseCache.set(expr, parsed);
  return parsed;
}

function fieldIdForName(name: string): FieldId {
  return BUILTIN_FIELD_NAMES.has(name) ? (name as FieldId) : `prop:${name}`;
}

export function formulaResultKey(row: string | FormulaEvalRow, formulaName: string): string {
  return `${typeof row === "string" ? row : formulaRowKey(row)}\0${formulaName}`;
}

export function readFormulaRowField(row: FormulaEvalRow, field: FieldId): FieldValue | null {
  if (field.startsWith("formula:")) return null;
  if (liveFormulaRowNode(row)) return readField(row.id, field);

  const f = row.dto ? facetsFromDto(row.dto) : null;
  if (!f) return null;
  switch (field) {
    case "state":
      return f.marker ? { text: f.marker, raw: f.marker } : null;
    case "priority":
      return f.priority ? { text: `[#${f.priority}]`, raw: f.priority } : null;
    case "scheduled":
      return f.scheduled ? { text: f.scheduled, raw: f.scheduled } : null;
    case "deadline":
      return f.deadline ? { text: f.deadline, raw: f.deadline } : null;
    case "tags":
      return f.tags.length ? { text: f.tags.map((t) => `#${t}`).join(" "), raw: f.tags.join(" ") } : null;
    case "page":
      return { text: row.page, raw: row.page };
    default: {
      const key = field.slice(5);
      const prop = f.properties.find(([k]) => k === key);
      return prop ? { text: prop[1], raw: prop[1] } : null;
    }
  }
}

export function fieldValueToFormulaValue(field: FieldId, value: FieldValue | null): FormulaValue {
  if (!value) return nullValue();
  if (field === "tags") {
    return {
      kind: "list",
      values: (value.raw ?? value.text)
        .split(/\s+/)
        .map((tag) => tag.trim())
        .filter(Boolean)
        .map(textValue),
    };
  }

  const raw = (value.raw ?? value.text).trim();
  if (field === "scheduled" || field === "deadline") {
    // Planning facets carry OG's day-name (and sometimes repeater) tail —
    // "2026-07-08 Wed" — which the strict date parser rejects; without this
    // the flagship `deadline < today()` comparison degrades to text.
    const iso = isoDatePrefix(raw);
    const date = iso ? parseDateValue(iso) : null;
    if (date) return date;
    return raw ? textValue(raw) : nullValue();
  }
  if (isPlainDecimalNumber(raw)) return numberValue(Number(raw));
  const date = parseDateValue(raw);
  if (date) return date;
  const lower = raw.toLowerCase();
  if (lower === "true") return booleanValue(true);
  if (lower === "false") return booleanValue(false);
  return textValue(value.raw ?? value.text);
}

function formulaAstResolver(formulas: ReadonlyMap<string, string>): (name: string) => Ast | null {
  return (name) => {
    const expr = formulas.get(name);
    if (expr == null) return null;
    const parsed = parseCached(expr);
    if (!parsed.ok) throw new Error(`Parse error at ${parsed.error.offset}: ${parsed.error.message}`);
    return parsed.ast;
  };
}

function evaluateAstForRow(
  row: FormulaEvalRow,
  ast: Ast,
  formulas: ReadonlyMap<string, string>,
  now: Date
): FormulaValue {
  const astFor = formulaAstResolver(formulas);
  return evaluate(ast, {
    field: (name) => fieldValueToFormulaValue(fieldIdForName(name), readFormulaRowField(row, fieldIdForName(name))),
    formulaAst: astFor,
    now,
  });
}

export function evaluateFormulaForRow(
  row: FormulaEvalRow,
  formulaName: string,
  formulas: ReadonlyMap<string, string>,
  now: Date
): FormulaValue {
  const astFor = formulaAstResolver(formulas);
  let ast: Ast | null;
  try {
    ast = astFor(formulaName);
  } catch (err) {
    return errorValue(`Formula ${formulaName} failed: ${err instanceof Error ? err.message : String(err)}`);
  }
  if (!ast) return errorValue(`Unknown formula ${formulaName}`);

  return evaluateAstForRow(row, ast, formulas, now);
}

function observeFormulaRow(row: FormulaEvalRow): void {
  const node = liveFormulaRowNode(row);
  if (node) {
    // Track the same fine-grained raw dependency ordinary sheet cells read.
    void facetsOf(node.raw, formatForBlock(row.id));
  }
}

export function createFormulaResultsMemo(opts: FormulaResultsOptions): Accessor<ReadonlyMap<string, FormulaValue>> {
  let warned = false;
  return createMemo(() => {
    dataRev();
    const rows = opts.rows();
    const formulas = opts.formulas();
    const names = [...formulas.keys()];
    const now = opts.now?.() ?? new Date();
    const out = new Map<string, FormulaValue>();
    let evaluations = 0;

    for (const row of rows) {
      observeFormulaRow(row);
      for (const name of names) {
        evaluations += 1;
        opts.onEvaluate?.();
        out.set(formulaResultKey(row, name), evaluateFormulaForRow(row, name, formulas, now));
      }
    }

    const threshold = opts.warnThreshold ?? DEFAULT_EVAL_WARN_THRESHOLD;
    if (evaluations > threshold && !warned) {
      warned = true;
      console.warn(
        `SheetTable ${opts.ownerId ?? ""} evaluated ${evaluations} formula cells in one render pass; consider reducing rows or formula columns.`
      );
    }
    return out;
  });
}

export function createFormulaFilterMemo<T extends FormulaEvalRow>(
  opts: FormulaFilterOptions<T>
): Accessor<FormulaFilterState<T>> {
  return createMemo(() => {
    dataRev();
    const rows = opts.rows();
    const expr = opts.filter()?.trim() ?? "";
    if (!expr) return { rows, error: null };

    const parsed = parseCached(expr);
    if (!parsed.ok) {
      return {
        rows,
        error: `Filter parse error at ${parsed.error.offset}: ${parsed.error.message}`,
      };
    }

    const formulas = opts.formulas();
    const now = opts.now?.() ?? new Date();
    const kept: T[] = [];
    for (const row of rows) {
      observeFormulaRow(row);
      const value = evaluateAstForRow(row, parsed.ast, formulas, now);
      if (value.kind === "boolean") {
        if (value.value) kept.push(row);
        continue;
      }
      const detail = value.kind === "error" ? value.message : `returned ${value.kind}`;
      return {
        rows,
        error: `Filter disabled${opts.ownerId ? ` for ${opts.ownerId}` : ""}: ${detail}`,
      };
    }

    return { rows: kept, error: null };
  });
}

export function formulaValueText(value: FormulaValue | null | undefined): string {
  if (!value || value.kind === "null") return "";
  switch (value.kind) {
    case "text":
      return value.value;
    case "number":
      return String(value.value);
    case "boolean":
      return value.value ? "true" : "false";
    case "date":
      return value.source;
    case "duration":
      return `${value.n}${value.unit}`;
    case "list":
      return value.values.map((item) => formulaValueText(item)).join(",");
    case "error":
      return value.message;
  }
}

export function formulaValueToFieldValue(value: FormulaValue | null | undefined): FieldValue | null {
  if (!value || value.kind === "null") return null;
  const text = formulaValueText(value);
  return { text, raw: text };
}
