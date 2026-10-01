import { isAggregateFn } from "./aggregate";
import { parseFields, sheetConfig, type FieldSpec } from "./config";
import { astToExpr, decodeFormulaExpr, encodeFormulaExpr, formulaNameValid, parseFormula, type Ast } from "./formula";
import { blockRegions } from "../render/parse";
import type { Format } from "../render/ast";

const PROPERTY_NAME = /^[A-Za-z_][A-Za-z0-9_-]*$/;
const FORMULA_LITERAL_NAMES = new Set(["true", "false", "null"]);
const BUILTIN_FIELDS = new Set(["state", "priority", "scheduled", "deadline", "tags", "page"]);
const PARTICIPATING_KEYS = new Set(["tine.fields", "tine.filter", "tine.group-by", "tine.col-aggregates"]);
const FORMULA_PREFIX = "tine.formula.";

export interface RenameSource {
  id: string;
  page: string;
  raw: string;
  format: Format;
  /** The exact lsdoc-backed facet projection for this raw source. */
  recognizedProperties: readonly (readonly [string, string])[];
}

export interface PlanSheetFieldRenameInput {
  rowSource: "children" | "query";
  ownerWritable: boolean;
  schemaHome: "block" | "page" | null;
  owner: RenameSource;
  rows: readonly RenameSource[];
  pageProperties?: readonly (readonly [string, string])[];
  recognizeProperties?: (raw: string, format: Format) => readonly (readonly [string, string])[];
  oldField: string;
  newName: string;
}

export interface RenameRawCandidate {
  id: string;
  raw: string;
}

export interface SheetFieldRenamePlan {
  ownerId: string;
  page: string;
  oldField: string;
  newField: string;
  ownerRaw: string;
  rows: readonly RenameRawCandidate[];
}

export type SheetFieldRenamePlanResult =
  | { ok: true; plan: SheetFieldRenamePlan }
  | { ok: false; error: string };



interface PropertyOccurrence {
  line: number;
  key: string;
  value: string;
  keyStart: number;
  keyEnd: number;
  valueStart: number;
  valueEnd: number;
}

interface Replacement {
  start: number;
  end: number;
  value: string;
}

function fail(error: string): SheetFieldRenamePlanResult {
  return { ok: false, error };
}

/** Accepted property key/value spans; UTF-8 transport coordinates are mapped
 * to the editor's UTF-16 without recognizing source grammar. Cost O(raw). */
export function propertyOccurrences(raw: string, format: Format): readonly PropertyOccurrence[] {
  const bytes = new TextEncoder().encode(raw);
  const decoder = new TextDecoder();
  let byte = 0, offset = 0, line = 0;
  const at = (next: number) => {
    const segment = decoder.decode(bytes.subarray(byte, next));
    offset += segment.length;
    line += segment.split("\n").length - 1;
    byte = next;
    return offset;
  };
  return blockRegions(raw, format).properties.filter((p) => p.primary).map((p) => {
    at(p.line[0]);
    const index = line;
    const keyStart = at(p.key_range[0]), keyEnd = at(p.key_range[1]);
    const valueStart = at(p.value_range[0]), valueEnd = at(p.value_range[1]);
    return { line: index, key: raw.slice(keyStart, keyEnd), value: p.value,
      keyStart, keyEnd, valueStart, valueEnd };
  });
}

function normalizedPair(key: string, value: string): string {
  return `${key.trim().toLowerCase()}\0${value.trim()}`;
}

function recognizedOccurrences(source: RenameSource): PropertyOccurrence[] | string {
  const occurrences = [...propertyOccurrences(source.raw, source.format)];
  const actual = occurrences.map((item) => normalizedPair(item.key, item.value));
  const recognized = source.recognizedProperties.map(([key, value]) => normalizedPair(key, value));
  if (actual.length !== recognized.length || actual.some((item, index) => item !== recognized[index])) {
    return `Cannot safely locate the canonical properties in ${source.id}; no changes were made.`;
  }
  return occurrences;
}

function replaceAll(raw: string, replacements: readonly Replacement[]): string {
  let out = raw;
  for (const replacement of [...replacements].sort((a, b) => b.start - a.start)) {
    out = out.slice(0, replacement.start) + replacement.value + out.slice(replacement.end);
  }
  return out;
}

/** Lossless raw-key helper used by the planner and directly regression-tested. */
export function renameCanonicalPropertyKey(
  raw: string,
  format: Format,
  oldName: string,
  newName: string,
): { ok: true; raw: string; count: number } | { ok: false; error: string } {
  const occurrences = propertyOccurrences(raw, format);
  const matches = occurrences.filter((item) => item.key === oldName);
  const variants = occurrences.filter((item) => item.key.toLowerCase() === oldName.toLowerCase());
  if (variants.length !== matches.length || matches.length > 1) {
    return { ok: false, error: `The property ${oldName} is duplicated or has ambiguous casing.` };
  }
  return {
    ok: true,
    count: matches.length,
    raw: replaceAll(raw, matches.map((item) => ({ start: item.keyStart, end: item.keyEnd, value: newName }))),
  };
}

function sameSpecsExceptRename(before: readonly FieldSpec[], after: readonly FieldSpec[], oldField: string, newField: string): boolean {
  if (before.length !== after.length) return false;
  return before.every((spec, index) => {
    const expected = spec.field === oldField ? newField : spec.field;
    return after[index].field === expected && JSON.stringify(after[index].type) === JSON.stringify(spec.type);
  });
}

export function rewriteSchemaValueLosslessly(
  value: string,
  oldName: string,
  newName: string,
): { ok: true; value: string } | { ok: false; error: string } {
  const segments = value.split(";");
  const matches: { index: number; start: number; end: number }[] = [];
  const variants: string[] = [];
  segments.forEach((segment, index) => {
    const eq = segment.indexOf("=");
    if (eq < 0) return;
    const left = segment.slice(0, eq);
    const name = left.trim();
    if (name.toLowerCase() !== oldName.toLowerCase()) return;
    variants.push(name);
    if (name === oldName) {
      const start = left.indexOf(name);
      matches.push({ index, start, end: start + name.length });
    }
  });
  if (variants.length !== 1 || matches.length !== 1) {
    return { ok: false, error: `The declared field ${oldName} is missing, duplicated, or has ambiguous casing.` };
  }
  const match = matches[0];
  segments[match.index] =
    segments[match.index].slice(0, match.start) + newName + segments[match.index].slice(match.end);
  const candidate = segments.join(";");
  if (!sameSpecsExceptRename(parseFields(value), parseFields(candidate), `prop:${oldName}`, `prop:${newName}`)) {
    return { ok: false, error: "The field schema could not be renamed losslessly." };
  }
  return { ok: true, value: candidate };
}

export function rewriteFieldAst(ast: Ast, oldName: string, newName: string): { ast: Ast; changed: boolean } {
  switch (ast.kind) {
    case "field":
      return ast.name === oldName ? { ast: { ...ast, name: newName }, changed: true } : { ast, changed: false };
    case "literal":
    case "formulaRef":
      return { ast, changed: false };
    case "unary": {
      const expr = rewriteFieldAst(ast.expr, oldName, newName);
      return expr.changed ? { ast: { ...ast, expr: expr.ast }, changed: true } : { ast, changed: false };
    }
    case "binary": {
      const left = rewriteFieldAst(ast.left, oldName, newName);
      const right = rewriteFieldAst(ast.right, oldName, newName);
      return left.changed || right.changed
        ? { ast: { ...ast, left: left.ast, right: right.ast }, changed: true }
        : { ast, changed: false };
    }
    case "call": {
      const args = ast.args.map((arg) => rewriteFieldAst(arg, oldName, newName));
      return args.some((arg) => arg.changed)
        ? { ast: { ...ast, args: args.map((arg) => arg.ast) }, changed: true }
        : { ast, changed: false };
    }
    case "member": {
      const object = rewriteFieldAst(ast.object, oldName, newName);
      const args = ast.args?.map((arg) => rewriteFieldAst(arg, oldName, newName)) ?? null;
      const changed = object.changed || !!args?.some((arg) => arg.changed);
      return changed
        ? { ast: { ...ast, object: object.ast, args: args?.map((arg) => arg.ast) ?? null }, changed: true }
        : { ast, changed: false };
    }
  }
}

function rewriteExpression(value: string, oldName: string, newName: string):
  | { ok: true; value: string; changed: boolean }
  | { ok: false; error: string } {
  const decoded = decodeFormulaExpr(value.trim());
  const parsed = parseFormula(decoded);
  if (!parsed.ok) return { ok: false, error: `${parsed.error.message} at ${parsed.error.offset}` };
  // I-22: both the rewriting visitor and deparser recurse. Admission here
  // bounds their stack before either walks an imported left-deep expression.
  const pending: { ast: Ast; depth: number }[] = [{ ast: parsed.ast, depth: 0 }];
  while (pending.length) {
    const { ast, depth } = pending.pop()!;
    if (depth >= 128) return { ok: false, error: "Formula depth exceeds 128 for field rename." };
    const children = ast.kind === "binary" ? [ast.left, ast.right]
      : ast.kind === "unary" ? [ast.expr]
      : ast.kind === "call" ? ast.args
      : ast.kind === "member" ? [ast.object, ...(ast.args ?? [])] : [];
    for (const child of children) pending.push({ ast: child, depth: depth + 1 });
  }
  const rewritten = rewriteFieldAst(parsed.ast, oldName, newName);
  const candidate = rewritten.changed ? replaceTrimmedValue(value, encodeFormulaExpr(astToExpr(rewritten.ast))) : value;
  const reparsed = parseFormula(decodeFormulaExpr(candidate.trim()));
  if (!reparsed.ok) return { ok: false, error: `Rewritten expression is invalid: ${reparsed.error.message}` };
  return {
    ok: true,
    changed: rewritten.changed,
    value: candidate,
  };
}

function rewriteGroupBy(value: string, oldName: string, newName: string): string | null {
  const parsed = sheetConfig([["tine.group-by", value]]).groupBy;
  if (parsed == null && value.trim()) return null;
  return parsed === `prop:${oldName}` ? replaceTrimmedValue(value, `prop:${newName}`) : value;
}

function replaceTrimmedValue(original: string, value: string): string {
  const start = original.search(/\S/);
  if (start < 0) return value;
  const end = original.search(/\s*$/);
  return original.slice(0, start) + value + original.slice(end);
}

interface AggregateSegmentShape {
  lead: string;
  key: string;
  mid: string;
  fn: string;
  trail: string;
}

/** The query grammar's own extra function word. Never widens `AggregateFn`. */
const QUERY_ONLY_AGGREGATE_FNS = new Set(["avg"]);

function aggregateSegmentShape(segment: string): AggregateSegmentShape | null {
  const match = /^(\s*)([^=;\s][^=;]*?)(\s*=\s*)([A-Za-z-]+)(\s*)$/.exec(segment);
  if (!match) return null;
  const fn = match[4].toLowerCase();
  if (!isAggregateFn(fn) && !QUERY_ONLY_AGGREGATE_FNS.has(fn)) return null;
  return { lead: match[1], key: match[2], mid: match[3], fn: match[4], trail: match[5] };
}

/** Whether an UNRECOGNIZED segment is nevertheless about the field being
 *  renamed. Preserving such a segment verbatim would leave a dangling reference
 *  to a name that no longer exists, and this helper cannot tell where the key
 *  ends inside it — so the rename refuses instead of guessing. */
function mentionsRenamedField(segment: string, oldName: string): boolean {
  return segment.toLowerCase().includes(`prop:${oldName.toLowerCase()}`);
}

/** **Rename one field inside `tine.col-aggregates`, losing nothing else** (P5A).
 *
 *  What changed from the first version, and why:
 *
 *   * a segment with no `=` used to fail the whole rename. A bare `count` is
 *     the query grammar's whole-result count, so a perfectly ordinary query
 *     configuration made renaming a sheet field impossible;
 *   * a repeated key used to fail. A query's aggregates are an ordered LIST —
 *     `prop:cost=sum;prop:cost=avg` asks for two footers on one column — so
 *     duplicates are retained, in order, and both get renamed;
 *   * `key=avg` used to fail, because `avg` is not in the sheet vocabulary. It
 *     is recognized here without being added to that vocabulary;
 *   * unrecognized segments used to fail. They are preserved verbatim unless
 *     they mention the field being renamed, which is the one case where
 *     preserving them would leave a dangling reference.
 *
 *  The refusals that REMAIN are the ones that make a rename genuinely
 *  ambiguous: an unparseable segment that names the old field, and a key that
 *  differs from `prop:<oldName>` only by case.
 *
 *  Only an EXACT `prop:<oldName>` key is renamed — bare query keys and
 *  `formula:` keys are outside the sheet's rename ownership and are left alone. */
export function rewriteAggregateValue(
  value: string,
  oldName: string,
  newName: string,
): { ok: true; value: string } | { ok: false; error: string } {
  const before = value.split(";");
  const after = [...before];
  const oldKey = `prop:${oldName}`;
  const newKey = `prop:${newName}`;
  for (let i = 0; i < before.length; i += 1) {
    const segment = before[i];
    if (!segment.trim()) continue;
    const shape = aggregateSegmentShape(segment);
    if (!shape) {
      if (mentionsRenamedField(segment, oldName)) {
        return { ok: false, error: "The column aggregate configuration is malformed or ambiguous." };
      }
      continue;
    }
    const key = shape.key.trim();
    if (key === oldKey) {
      const keyStart = segment.indexOf(shape.key) + shape.key.indexOf(key);
      after[i] = segment.slice(0, keyStart) + newKey + segment.slice(keyStart + key.length);
    } else if (key.toLowerCase() === oldKey.toLowerCase()) {
      return { ok: false, error: "The column aggregate configuration has ambiguous field casing." };
    }
  }
  // **The preservation proof is an ORDERED SEGMENT COMPARISON.** It used to be
  // `sheetConfig(...).colAggregates.size`, which is a `Map` keyed on the
  // aggregate key — the very shape that collapses repeated keys and drops bare
  // and unrecognized segments. Comparing its size therefore proved nothing
  // about exactly the segments this rename touches. Every segment must survive
  // in place, byte for byte, with one permitted difference: the key that was
  // exactly `prop:<oldName>` is now exactly `prop:<newName>`.
  if (before.length !== after.length) {
    return { ok: false, error: "The column aggregate configuration could not be preserved." };
  }
  for (let i = 0; i < before.length; i += 1) {
    if (before[i] === after[i]) continue;
    const b = aggregateSegmentShape(before[i]);
    const a = aggregateSegmentShape(after[i]);
    const intended = b !== null
      && a !== null
      && b.lead === a.lead
      && b.mid === a.mid
      && b.fn === a.fn
      && b.trail === a.trail
      && b.key.trim() === oldKey
      && a.key.trim() === newKey;
    if (!intended) {
      return { ok: false, error: "The column aggregate configuration could not be preserved." };
    }
  }
  return { ok: true, value: after.join(";") };
}

function formulaEntriesFromOccurrences(
  occurrences: readonly PropertyOccurrence[],
): { entries: Map<string, PropertyOccurrence>; error?: string } {
  const entries = new Map<string, PropertyOccurrence>();
  for (const occurrence of occurrences) {
    const lower = occurrence.key.toLowerCase();
    if (!lower.startsWith(FORMULA_PREFIX)) continue;
    const name = occurrence.key.slice(FORMULA_PREFIX.length).trim();
    if (!formulaNameValid(name)) continue;
    const identity = name.toLowerCase();
    if (entries.has(identity)) return { entries, error: `Formula ${name} is duplicated or has ambiguous casing.` };
    entries.set(identity, occurrence);
  }
  return { entries };
}

function pageFormulaEntries(
  props: readonly (readonly [string, string])[],
): { entries: Map<string, string>; error?: string } {
  const entries = new Map<string, string>();
  for (const [rawKey, rawValue] of props) {
    const key = rawKey.trim();
    if (!key.toLowerCase().startsWith(FORMULA_PREFIX)) continue;
    const name = key.slice(FORMULA_PREFIX.length).trim();
    if (!formulaNameValid(name)) continue;
    const identity = name.toLowerCase();
    if (entries.has(identity)) return { entries, error: `Page formula ${name} is duplicated or has ambiguous casing.` };
    entries.set(identity, rawValue);
  }
  return { entries };
}

function uniqueConfigOccurrences(occurrences: readonly PropertyOccurrence[]):
  | { ok: true; byKey: Map<string, PropertyOccurrence> }
  | { ok: false; error: string } {
  const byKey = new Map<string, PropertyOccurrence>();
  for (const occurrence of occurrences) {
    const key = occurrence.key.toLowerCase();
    if (!PARTICIPATING_KEYS.has(key) && !key.startsWith(FORMULA_PREFIX)) continue;
    if (byKey.has(key)) return { ok: false, error: `The owner has duplicate ${occurrence.key} properties.` };
    byKey.set(key, occurrence);
  }
  return { ok: true, byKey };
}

function exactPropertyDelta(
  before: readonly PropertyOccurrence[],
  after: readonly PropertyOccurrence[],
  oldName: string,
  newName: string,
): boolean {
  if (before.length !== after.length) return false;
  return before.every((item, index) => {
    const expectedKey = item.key === oldName ? newName : item.key;
    return after[index].key === expectedKey && after[index].value === item.value;
  });
}

export function planSheetFieldRename(input: PlanSheetFieldRenameInput): SheetFieldRenamePlanResult {
  if (input.rowSource !== "children") return fail("Rename is available only for children-backed tables.");
  if (!input.ownerWritable) return fail("This table is read-only.");
  if (input.schemaHome !== "block") return fail("Rename requires a block-local field schema.");
  if (!input.oldField.startsWith("prop:")) return fail("Only declared property fields can be renamed.");
  const oldName = input.oldField.slice("prop:".length);
  const newName = input.newName.trim();
  if (!PROPERTY_NAME.test(newName)) {
    return fail("Field names must start with a letter or underscore and contain only letters, numbers, _ or -.");
  }
  if (FORMULA_LITERAL_NAMES.has(newName)) return fail(`${newName} is reserved by the formula language.`);
  if (newName === oldName) return fail("Enter a different field name.");
  if (BUILTIN_FIELDS.has(newName.toLowerCase())) return fail(`${newName} is a built-in field name.`);
  if (input.rows.some((row) => row.page !== input.owner.page)) {
    return fail("All direct rows must belong to the table owner's page.");
  }

  const ownerOccurrencesResult = recognizedOccurrences(input.owner);
  if (typeof ownerOccurrencesResult === "string") return fail(ownerOccurrencesResult);
  const ownerOccurrences = ownerOccurrencesResult;
  const uniqueConfig = uniqueConfigOccurrences(ownerOccurrences);
  if (!uniqueConfig.ok) return fail(uniqueConfig.error);
  const fieldsOccurrence = uniqueConfig.byKey.get("tine.fields");
  if (!fieldsOccurrence) return fail("The table does not have a block-local tine.fields schema.");

  const declared = parseFields(fieldsOccurrence.value);
  if (!declared.some((spec) => spec.field === input.oldField && spec.type !== "builtin")) {
    return fail(`${oldName} is not a declared property field.`);
  }
  for (const spec of declared) {
    if (spec.field === input.oldField) continue;
    const name = spec.field.startsWith("prop:") ? spec.field.slice(5) : spec.field;
    if (name.toLowerCase() === newName.toLowerCase()) return fail(`A declared field already uses ${newName}.`);
  }
  for (const segment of fieldsOccurrence.value.split(";")) {
    const eq = segment.indexOf("=");
    if (eq < 0) continue;
    const name = segment.slice(0, eq).trim();
    if (name === oldName) continue;
    if (name.toLowerCase() === newName.toLowerCase()) {
      return fail(`The field schema already contains ${newName}, including in an unrecognized segment.`);
    }
  }

  const rowOccurrences = new Map<string, PropertyOccurrence[]>();
  for (const row of input.rows) {
    const result = recognizedOccurrences(row);
    if (typeof result === "string") return fail(result);
    const oldVariants = result.filter((item) => item.key.toLowerCase() === oldName.toLowerCase());
    const exactOld = oldVariants.filter((item) => item.key === oldName);
    if (oldVariants.length !== exactOld.length || exactOld.length > 1) {
      return fail(`Row ${row.id} has a duplicated or ambiguously-cased ${oldName} property.`);
    }
    if (result.some((item) => item.key !== oldName && item.key.toLowerCase() === newName.toLowerCase())) {
      return fail(`Row ${row.id} already has a ${newName} property.`);
    }
    rowOccurrences.set(row.id, result);
  }

  const schema = rewriteSchemaValueLosslessly(fieldsOccurrence.value, oldName, newName);
  if (!schema.ok) return fail(schema.error);
  const replacements: Replacement[] = [{
    start: fieldsOccurrence.valueStart,
    end: fieldsOccurrence.valueEnd,
    value: schema.value,
  }];

  const localFormulaResult = formulaEntriesFromOccurrences(ownerOccurrences);
  if (localFormulaResult.error) return fail(localFormulaResult.error);
  const pageFormulaResult = pageFormulaEntries(input.pageProperties ?? []);
  if (pageFormulaResult.error) return fail(pageFormulaResult.error);

  for (const [name, value] of pageFormulaResult.entries) {
    if (localFormulaResult.entries.has(name)) continue;
    const rewritten = rewriteExpression(value, oldName, newName);
    if (!rewritten.ok) return fail(`Page formula ${name} cannot be parsed: ${rewritten.error}`);
    if (rewritten.changed) return fail(`Formula ${name} is page-owned; graph-wide field rename is not available yet.`);
  }
  for (const [name, occurrence] of localFormulaResult.entries) {
    const rewritten = rewriteExpression(occurrence.value, oldName, newName);
    if (!rewritten.ok) return fail(`Formula ${name} cannot be parsed: ${rewritten.error}`);
    if (rewritten.changed) replacements.push({ start: occurrence.valueStart, end: occurrence.valueEnd, value: rewritten.value });
  }

  const filter = uniqueConfig.byKey.get("tine.filter");
  if (filter) {
    const rewritten = rewriteExpression(filter.value, oldName, newName);
    if (!rewritten.ok) return fail(`The table filter cannot be parsed: ${rewritten.error}`);
    if (rewritten.changed) replacements.push({ start: filter.valueStart, end: filter.valueEnd, value: rewritten.value });
  }

  const groupBy = uniqueConfig.byKey.get("tine.group-by");
  if (groupBy) {
    const value = rewriteGroupBy(groupBy.value, oldName, newName);
    if (value == null) return fail("The group-by configuration is malformed.");
    if (value !== groupBy.value) replacements.push({ start: groupBy.valueStart, end: groupBy.valueEnd, value });
  }

  const aggregates = uniqueConfig.byKey.get("tine.col-aggregates");
  if (aggregates) {
    const rewritten = rewriteAggregateValue(aggregates.value, oldName, newName);
    if (!rewritten.ok) return fail(rewritten.error);
    if (rewritten.value !== aggregates.value) {
      replacements.push({ start: aggregates.valueStart, end: aggregates.valueEnd, value: rewritten.value });
    }
  }

  const ownerRaw = replaceAll(input.owner.raw, replacements);
  const ownerAfter = propertyOccurrences(ownerRaw, input.owner.format);
  if (ownerAfter.length !== ownerOccurrences.length) return fail("The owner rewrite did not preserve its property region.");
  if (input.recognizeProperties) {
    const parsed = input.recognizeProperties(ownerRaw, input.owner.format).map(([key, value]) => normalizedPair(key, value));
    const located = ownerAfter.map((item) => normalizedPair(item.key, item.value));
    if (parsed.length !== located.length || parsed.some((item, index) => item !== located[index])) {
      return fail("The renamed owner does not round-trip through the canonical property parser.");
    }
  }
  const fieldsAfter = ownerAfter.find((item) => item.key.toLowerCase() === "tine.fields");
  if (!fieldsAfter || !sameSpecsExceptRename(declared, parseFields(fieldsAfter.value), input.oldField, `prop:${newName}`)) {
    return fail("The owner rewrite changed more than the requested field identity.");
  }

  const rows: RenameRawCandidate[] = [];
  for (const row of input.rows) {
    const before = rowOccurrences.get(row.id)!;
    const renamed = renameCanonicalPropertyKey(row.raw, row.format, oldName, newName);
    if (!renamed.ok) return fail(renamed.error);
    if (renamed.count === 0) continue;
    const after = propertyOccurrences(renamed.raw, row.format);
    if (!exactPropertyDelta(before, after, oldName, newName)) {
      return fail(`The row ${row.id} rewrite changed more than its property key.`);
    }
    if (input.recognizeProperties) {
      const parsed = input.recognizeProperties(renamed.raw, row.format).map(([key, value]) => normalizedPair(key, value));
      const located = after.map((item) => normalizedPair(item.key, item.value));
      if (parsed.length !== located.length || parsed.some((item, index) => item !== located[index])) {
        return fail(`The renamed row ${row.id} does not round-trip through the canonical property parser.`);
      }
    }
    rows.push({ id: row.id, raw: renamed.raw });
  }

  return {
    ok: true,
    plan: {
      ownerId: input.owner.id,
      page: input.owner.page,
      oldField: input.oldField,
      newField: `prop:${newName}`,
      ownerRaw,
      rows,
    },
  };
}
