// Test support for queryIr.test.ts and queryBuilder.test.ts: the mirror's
// vocabulary lists and its exhaustive visitor, "the mirror's own proof that it
// knows every variant". Production never reads them, so they live beside the
// tests, not in queryIr.ts.
import type {
  Anchor, Attr, CmpOp, DiagnosticKind, Filter, Leaf, NearDir, Query, QueryStatistics, Quant, Rel, Value,
} from "./queryIr";

export const ANCHORS: readonly Anchor[] = ["block", "page"];
export const ATTRS: readonly Attr[] = [
  "content", "task", "priority", "scheduled", "deadline", "created_at", "last_modified_at",
  "name", "journal", "day", "namespace", "used_as_tag",
  "key", "value", "atom_count",
];
export const RELS: readonly Rel[] = ["refs", "direct_refs", "tags", "props", "children", "parent", "ancestors", "descendants", "blocks", "page", "links", "linked_by"];
export const NEAR_DIRS: readonly NearDir[] = ["out", "in", "both"];
export const QUANTS: readonly Quant[] = ["any", "none", "every"];
export const CMP_OPS: readonly CmpOp[] = [
  "eq", "not_eq", "lt", "le", "gt", "ge",
  "between", "in", "not_in",
  "like", "starts_with", "match", "regex",
  "is_set", "is_not_set", "is_blank",
];
export const DIAGNOSTIC_KINDS: readonly DiagnosticKind[] = [
  "unknown_head", "syntax", "unknown_ident", "not_applicable", "depth", "size",
];

/** A query with at least one ENABLED diagnostic is invalid: it returns zero results plus its diagnostics (§3.5). */
export function isInvalid(query: Query): boolean {
  return (query.diagnostics ?? []).some((d) => !d.disabled);
}

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
    case "near":
      if (leaf.dir !== undefined && !NEAR_DIRS.includes(leaf.dir)) throw new UnknownIrVariantError("NearDir", leaf.dir);
      forEachFilter(leaf.seeds, visit);
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
    if (leaf.kind === "near") return;
    if (!RELS.includes(leaf.rel)) throw new UnknownIrVariantError("Rel", leaf.rel);
    if (!QUANTS.includes(leaf.quant)) throw new UnknownIrVariantError("Quant", leaf.quant);
  });
}

