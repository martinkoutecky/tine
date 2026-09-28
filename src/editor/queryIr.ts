// **The TypeScript mirror of the Rust query IR** (SPEC §3.1, §7.1).

import type { PageKind, RefGroup } from "../types";
/** Display drafts are typed by Q4b's `queryDisplayDraft`; opaque here so this mirror stays the one seam. */
export type QueryDisplayDraft = Record<string, unknown>;
export type FriendlyPageMatchScope = string;

// Scalars

/** A source span, in UTF-16 code units into the original source text. */
export interface Span {
  start: number;
  end: number;
}

/** The row type a query selects. Required in the IR and enforced by Tine (Q4). */
export type Anchor = "block" | "page";
export const ANCHORS: readonly Anchor[] = ["block", "page"];

/** The attributes of a block row, of a page row, and the three attributes of the elements relations yield. */
export type Attr =
  // block row
  | "content" | "task" | "priority" | "scheduled" | "deadline"
  // page row
  | "name" | "journal" | "day" | "namespace"
  // property element
  | "key" | "value" | "atom_count";
export const ATTRS: readonly Attr[] = [
  "content", "task", "priority", "scheduled", "deadline",
  "name", "journal", "day", "namespace",
  "key", "value", "atom_count",
];

/** One relation of the anchor row. */
export type Rel = "refs" | "tags" | "props" | "children" | "blocks" | "page";
export const RELS: readonly Rel[] = ["refs", "tags", "props", "children", "blocks", "page"];

/** OData §5.1.1.13 quantifiers: `any` is false and `every` true on an empty collection (Q5). */
export type Quant = "any" | "none" | "every";
export const QUANTS: readonly Quant[] = ["any", "none", "every"];

/** The complete comparison vocabulary (§3.3, M19). */
export type CmpOp =
  | "eq" | "not_eq" | "lt" | "le" | "gt" | "ge"
  | "between" | "in" | "not_in"
  | "like" | "starts_with"
  /** Full-text match on `content` — what today's `(search "…")` head means. */
  | "match"
  /** A Rust regex over `content` — Tine's `(content-regex "…")`. Never OG-expressible. */
  | "regex"
  | "is_set" | "is_not_set" | "is_blank";
export const CMP_OPS: readonly CmpOp[] = [
  "eq", "not_eq", "lt", "le", "gt", "ge",
  "between", "in", "not_in",
  "like", "starts_with", "match", "regex",
  "is_set", "is_not_set", "is_blank",
];

/** Why a query is (partly) not understood. */
export type DiagnosticKind =
  | "unknown_head" | "syntax" | "unknown_ident" | "not_applicable" | "depth" | "size";
export const DIAGNOSTIC_KINDS: readonly DiagnosticKind[] = [
  "unknown_head", "syntax", "unknown_ident", "not_applicable", "depth", "size",
];

// Values, leaves, the boolean tree

/** A comparison operand. */
export type Value =
  | { kind: "text"; text: string }
  | { kind: "number"; number: number }
  | { kind: "date"; literal: string }
  | { kind: "bool"; bool: boolean }
  | { kind: "list"; items: Value[] }
  /** The operand of `is_set` / `is_not_set` / `is_blank` and of nothing else. */
  | { kind: "none" };

/** A self-contained test on the current row. */
export type Leaf =
  /** A comparison on one of the current row's own attributes. */
  | { kind: "attr"; attr: Attr; op: CmpOp; value: Value }
  /** A quantifier over ONE relation of the current row. */
  | { kind: "rel"; rel: Rel; quant: Quant; pred: Filter };

/** The boolean tree. */
export type Filter =
  | { kind: "and"; items: Filter[] }
  | { kind: "or"; items: Filter[] }
  | { kind: "not"; inner: Filter }
  | { kind: "leaf"; leaf: Leaf }
  /** Q12: present, round-trips, structurally omitted at evaluation (§3.5). */
  | { kind: "off"; inner: Filter }
  /** An unparsed or unknown span. */
  | { kind: "raw"; text: string; diagnostic_kind: DiagnosticKind; span?: Span }
  | { kind: "true" }
  | { kind: "false" };

export interface Diagnostic {
  span?: Span;
  message: string;
  suggestions?: string[];
  /** Set for a diagnostic inside an `off` subtree: the row renders greyed with
  *  its message but does NOT invalidate the query (§3.5). */
  disabled?: boolean;
  kind: DiagnosticKind;
}

/** Where the query's text came from, and the bytes to preserve. */
export type Source =
  | { kind: "og"; original: string; og_options?: string }
  | { kind: "tql"; original: string; og_options?: string }
  /** Datalog. */
  | { kind: "advanced"; original: string; og_options?: string }
  /** Built in the UI: no authored text to preserve. */
  | { kind: "builder" };

/** The opaque trailing options map, or `""`. */
export function sourceOptions(source: Source): string {
  return source.kind === "builder" ? "" : source.og_options ?? "";
}

/** The exact authored form slice, without the trailing options map, or null for a builder-authored query. */
export function sourceOriginal(source: Source): string | null {
  return source.kind === "builder" ? null : source.original;
}

// View settings, bounds, the query

export type SortDir = "asc" | "desc";
/** `search` is the existing fourth view (`Macro.tsx`), and it stays. */
export type ViewKind = "search" | "list" | "table" | "board";
export const VIEW_KINDS: readonly ViewKind[] = ["search", "list", "table", "board"];
export type AggFn = "count" | "sum" | "avg";

/** A sort/group/column/aggregate target: a property key or an OG-sortable field name, kept as the user wrote it. */
export type Field = string;

/** Presentation. */
export interface ViewSettings {
  view?: ViewKind;
  sort?: [Field, SortDir][];
  group_by?: Field;
  columns?: Field[];
  aggregates?: [Field, AggFn][];
  sample?: number;
}

/** The two construction limits the result bridge already enforces, and nothing else. */
export interface Bounds {
  max_rows: number;
  max_bytes: number;
}

/** One query: an anchored filter plus the diagnostics and the authored source. */
export interface Query {
  anchor: Anchor;
  filter: Filter;
  diagnostics?: Diagnostic[];
  source: Source;
}

/** Page/block presentation state read from a saved query's scoped `tine.*` properties. */
export interface ScopedDisplaySettings {
  page_presentation?: ViewKind;
  page_display?: QueryDisplayDraft;
  block_presentation?: ViewKind;
  block_display?: QueryDisplayDraft;
  page_match_scope?: FriendlyPageMatchScope;
  unreadable_settings?: string[];
}

/** The Display half of a `run_graph_search` request (§7.6, Q3). */
export interface GraphSearchDisplayOptions {
  pageMatchScope?: FriendlyPageMatchScope;
  pageView?: ViewSettings;
  blockView?: ViewSettings;
}

export type GraphSearchConsumer = "non_interactive" | "ctrl_k" | "search_tab";

/** `query_parse`'s answer. */
export interface ParsedQuery extends ScopedDisplaySettings {
  query: Query;
  view: ViewSettings;
}

/** A query with at least one ENABLED diagnostic is invalid: it returns zero results plus its diagnostics (§3.5). */
export function isInvalid(query: Query): boolean {
  return (query.diagnostics ?? []).some((d) => !d.disabled);
}

// Results

/** One `@page` result row. Needs no document load (K16). */
export interface PageRow {
  path: string;
  name: string;
  kind: PageKind;
  journal_day?: number;
  properties: [string, string][];
}

/** The advanced-query report (M5): OG/TQL sources report an empty `ignored` and `supported: true`. */
export interface QueryReport {
  ran?: string[];
  ignored?: string[];
  supported: boolean;
}

/** The anchor-specific result rows (§7.1, K16), flattened onto the result. */
export type QueryStatisticsMarker = "non_finite" | "division_by_zero" | "empty_group" | "non_numeric";
export type QueryStatisticsCell =
  | { kind: "number"; value: number; skipped: number }
  | { kind: "marker"; reason: QueryStatisticsMarker; skipped: number };
export type QueryStatisticsGroup = { key: string | null; count: number; cells: QueryStatisticsCell[] };
export type QueryStatistics = {
  count: number; aggregates: [Field, AggFn][]; group_by: Field | null;
  overall: QueryStatisticsCell[]; groups: QueryStatisticsGroup[] | null;
  grouping_status: "none" | "exact" | "unsupported_formula";
};

export function isQueryStatistics(value: unknown): value is QueryStatistics {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  const count = (n: unknown) => typeof n === "number" && Number.isSafeInteger(n) && n >= 0;
  const cell = (c: unknown): boolean => {
    if (!c || typeof c !== "object") return false;
    const x = c as Record<string, unknown>;
    if (!count(x.skipped)) return false;
    if (x.kind === "number") return Object.keys(x).length === 3 && typeof x.value === "number" && Number.isFinite(x.value);
    return x.kind === "marker" && Object.keys(x).length === 3 &&
      ["non_finite", "division_by_zero", "empty_group", "non_numeric"].includes(x.reason as string);
  };
  if (!count(v.count) || !Array.isArray(v.aggregates) || !v.aggregates.every((a) =>
    Array.isArray(a) && a.length === 2 && typeof a[0] === "string" && ["count", "sum", "avg"].includes(a[1]))) return false;
  const cells = (a: unknown) => Array.isArray(a) && a.length === (v.aggregates as unknown[]).length && a.every(cell);
  if (!(v.group_by === null || typeof v.group_by === "string") || !cells(v.overall)) return false;
  if (v.grouping_status === "none" || v.grouping_status === "unsupported_formula") return v.groups === null;
  return v.grouping_status === "exact" && Array.isArray(v.groups) && v.groups.every((g) =>
    g && typeof g === "object" && (g.key === null || typeof g.key === "string") && count(g.count) && cells(g.cells));
}

export type QueryResult = {
  statistics?: QueryStatistics;
  diagnostics?: Diagnostic[];
  report: QueryReport;
  total: number;
  /** Exact complete count when the backend proved one for this row kind. */
  matched_total?: number;
  exceeded: boolean;
} & (
  | { anchor: "block"; groups: RefGroup[] }
  | { anchor: "page"; pages: PageRow[] }
);

/** The runtime inputs an execution binds against (§4.4, §7.1, R5). */
export interface ExecutionContext {
  current_page?: string;
}

/** One row of `query_explain_empty` (Q14, N19): a top-level conjunct, the anchor
*  rows matching it ALONE, and the rows matching all the OTHERS without it. */
export interface EmptyExplanation {
  conjunct: string;
  alone: number;
  without?: number;
}

/** `query_explain_empty`'s complete answer. */
export interface ExplainEmptyResult {
  rows: EmptyExplanation[];
  diagnostics?: Diagnostic[];
  report: QueryReport;
}

// The property registry (§6.1)

export type ObservedType = "text" | "number" | "date" | "checkbox" | "ref";
export type Cardinality = "one" | "many";

export interface RegistryRow {
  normalized_name: string;
  cardinality: Cardinality;
  observed_type: ObservedType;
  count_blocks: number;
  count_pages: number;
  /** Atom counts per class. */
  histogram?: [ObservedType, number][];
  mismatch_count: number;
  declared?: [ObservedType, Cardinality];
  /** At most eight, by count. */
  top_values?: [string, number][];
}

export interface RegistrySnapshot {
  rows: RegistryRow[];
  generation: number;
}

// The exhaustive visitor — the mirror's own proof that it knows every variant

/** Thrown when a value arrives carrying a `kind` this mirror does not know. */
export class UnknownIrVariantError extends Error {
  constructor(readonly where: string, readonly value: unknown) {
    super(
      `Unknown query IR variant in ${where}: ${JSON.stringify(value)}.\n`
      + `The Rust IR (crates/tine-core/src/query/ir.rs) has a shape this mirror `
      + `(src/editor/queryIr.ts) does not know. Add it to BOTH, and to the golden `
      + `fixtures in crates/tine-core/tests/fixtures/query-ir/ that pin the pair.`,
    );
    this.name = "UnknownIrVariantError";
  }
}

/** Walk every node of a filter tree, depth-first, in wire order. */
export function forEachFilter(filter: Filter, visit: (node: Filter) => void): void {
  visit(filter);
  switch (filter.kind) {
    case "and":
    case "or":
      for (const item of filter.items) forEachFilter(item, visit);
      return;
    case "not":
    case "off":
      forEachFilter(filter.inner, visit);
      return;
    case "leaf":
      forEachLeafFilter(filter.leaf, visit);
      return;
    case "raw":
    case "true":
    case "false":
      return;
    default:
      throw new UnknownIrVariantError("Filter", filter);
  }
}

function forEachLeafFilter(leaf: Leaf, visit: (node: Filter) => void): void {
  switch (leaf.kind) {
    case "attr":
      assertKnownValue(leaf.value);
      return;
    case "rel":
      forEachFilter(leaf.pred, visit);
      return;
    default:
      throw new UnknownIrVariantError("Leaf", leaf);
  }
}

function assertKnownValue(value: Value): void {
  switch (value.kind) {
    case "text":
    case "number":
    case "date":
    case "bool":
    case "none":
      return;
    case "list":
      for (const item of value.items) assertKnownValue(item);
      return;
    default:
      throw new UnknownIrVariantError("Value", value);
  }
}

/** Validate a whole parsed query against this mirror, throwing on the first variant it does not know. */
export function assertMirrorsIr(query: Query): void {
  switch (query.source.kind) {
    case "og":
    case "tql":
    case "advanced":
    case "builder":
      break;
    default:
      throw new UnknownIrVariantError("Source", query.source);
  }
  if (!ANCHORS.includes(query.anchor)) {
    throw new UnknownIrVariantError("Anchor", query.anchor);
  }
  for (const diagnostic of query.diagnostics ?? []) {
    if (!DIAGNOSTIC_KINDS.includes(diagnostic.kind)) {
      throw new UnknownIrVariantError("DiagnosticKind", diagnostic.kind);
    }
  }
  forEachFilter(query.filter, (node) => {
    if (node.kind !== "leaf") return;
    const leaf = node.leaf;
    if (leaf.kind === "attr") {
      if (!ATTRS.includes(leaf.attr)) throw new UnknownIrVariantError("Attr", leaf.attr);
      if (!CMP_OPS.includes(leaf.op)) throw new UnknownIrVariantError("CmpOp", leaf.op);
      return;
    }
    if (!RELS.includes(leaf.rel)) throw new UnknownIrVariantError("Rel", leaf.rel);
    if (!QUANTS.includes(leaf.quant)) throw new UnknownIrVariantError("Quant", leaf.quant);
  });
}

// Command dialects (SPEC §7.1, §4.3)

/** What `query_parse` is being handed. */
export type QueryTextDialect = "og" | "tql" | "advanced" | "macro_query" | "macro_tql";

/** The printed form a `query_print` caller wants (§4.3). */
export type QueryPrintDialect = "og" | "tql" | "tql_macro" | "advanced_macro";

/** The parse dialect for a macro of this name (§7.1): `{{query …}}` carries OG
*  or advanced text, `{{tine-query …}}` carries TQL. The ONE mapping, so a
*  caller cannot pick a dialect that contradicts the name it is about to write. */
export function macroTextDialect(macroName: string): QueryTextDialect {
  return macroName.toLowerCase() === "tine-query" ? "macro_tql" : "macro_query";
}

/** The print dialect that re-emits a macro of this name. */
export function macroPrintDialect(macroName: string): QueryPrintDialect {
  return macroName.toLowerCase() === "tine-query" ? "tql_macro" : "og";
}

/** The print dialect that re-emits the query the way it was AUTHORED. */
export function sourcePrintDialect(source: Source): QueryPrintDialect {
  switch (source.kind) {
    case "advanced":
      return "advanced_macro";
    case "tql":
      return "tql_macro";
    case "og":
    case "builder":
      return "og";
  }
}
