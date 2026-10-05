import { createMemo, type Accessor } from "solid-js";
import { dataRev } from "../graphSession";
import { formatForBlock, pageByName, type Node as StoreNode, node as docNode } from "../document";
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
import { fieldValueFromFacets, readField, type FieldId, type FieldValue } from "./fields";
import type { BlockDto, PageKind } from "../types";

import { appNow } from "../journal";
export interface FormulaEvalRow {
  id: string;
  page: string;
  kind?: PageKind;
  dto?: BlockDto;
  /** Evaluate from `dto` only, never the live document store: the static export
   *  computes a page it may not have loaded, and must not read a different one. */
  detached?: boolean;
}

export function formulaRowKey(row: Pick<FormulaEvalRow, "id" | "page" | "kind">): string {
  return row.kind ? `${row.kind}\0${row.page}\0${row.id}` : row.id;
}

export function liveFormulaRowNode(row: FormulaEvalRow): StoreNode | null {
  if (row.detached) return null;
  const node = docNode(row.id);
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
  return fieldValueFromFacets(f, field, row.page);
}

export function fieldValueToFormulaValue(field: FieldId, value: FieldValue | null): FormulaValue {
  if (!value) return nullValue();
  if (field === "tags") {
    return {
      kind: "list",
      values: (value.items ?? (value.raw ?? value.text).split(/\s+/))
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

/** Every formula value for every row: O(rows x formulas). The ONE evaluator loop
 *  for the live tables/boards and the static export; `onEvaluate` counts cells. */
export function computeFormulaResults(
  rows: readonly FormulaEvalRow[],
  formulas: ReadonlyMap<string, string>,
  now: Date,
  onEvaluate?: () => void
): { results: Map<string, FormulaValue>; evaluations: number } {
  const names = [...formulas.keys()];
  const results = new Map<string, FormulaValue>();
  let evaluations = 0;
  for (const row of rows) {
    observeFormulaRow(row);
    for (const name of names) {
      evaluations += 1;
      onEvaluate?.();
      results.set(formulaResultKey(row, name), evaluateFormulaForRow(row, name, formulas, now));
    }
  }
  return { results, evaluations };
}

export function createFormulaResultsMemo(opts: FormulaResultsOptions): Accessor<ReadonlyMap<string, FormulaValue>> {
  let warned = false;
  return createMemo(() => {
    dataRev();
    const { results, evaluations } = computeFormulaResults(
      opts.rows(),
      opts.formulas(),
      opts.now?.() ?? appNow(),
      opts.onEvaluate
    );
    const threshold = opts.warnThreshold ?? DEFAULT_EVAL_WARN_THRESHOLD;
    if (evaluations > threshold && !warned) {
      warned = true;
      console.warn("SheetTable evaluated many formula cells in one render pass");
    }
    return results;
  });
}

/** The rows a `tine.filter` keeps, or the filter's error with every row kept.
 *  Cost O(rows x expression); the ONE filter answer for the live tables/boards
 *  and the static export. */
export function filterFormulaRows<T extends FormulaEvalRow>(
  rows: readonly T[],
  expr: string | null,
  formulas: ReadonlyMap<string, string>,
  now: Date,
  ownerId?: string
): FormulaFilterState<T> {
  const source = expr?.trim() ?? "";
  if (!source) return { rows, error: null };

  const parsed = parseCached(source);
  if (!parsed.ok) {
    return {
      rows,
      error: `Filter parse error at ${parsed.error.offset}: ${parsed.error.message}`,
    };
  }

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
      error: `Filter disabled${ownerId ? ` for ${ownerId}` : ""}: ${detail}`,
    };
  }

  return { rows: kept, error: null };
}

export function createFormulaFilterMemo<T extends FormulaEvalRow>(
  opts: FormulaFilterOptions<T>
): Accessor<FormulaFilterState<T>> {
  return createMemo(() => {
    dataRev();
    return filterFormulaRows(opts.rows(), opts.filter(), opts.formulas(), opts.now?.() ?? appNow(), opts.ownerId);
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
