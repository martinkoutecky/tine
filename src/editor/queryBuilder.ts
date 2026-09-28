// The visual builder's model — **over the IR, not over text** (SPEC §7.1, §7.4).

import { MARKERS as TASK_MARKERS } from "../markers";
import type {
  Anchor,
  Attr,
  Cardinality,
  CmpOp,
  Filter,
  Leaf,
  ObservedType,
  Quant,
  Rel,
  Value,
} from "./queryIr";

// Vocabulary the pickers offer

/** Which date a `between` row tests against. */
export type BetweenField = "any" | "journal" | "scheduled" | "deadline";
export const BETWEEN_FIELDS: BetweenField[] = ["journal", "scheduled", "deadline", "any"];

/** The full task-marker set (src/markers.ts) as a mutable array for the picker. */
export const MARKERS: string[] = [...TASK_MARKERS];
export const PRIORITIES = ["A", "B", "C"];

/** **How many levels of the tree the builder RENDERS (SPEC §7.4).**
*
*  This is a presentation cap, not a language limit: the parser's
*  `QUERY_NESTING_MAX` is still 64 and a deeper query still parses, still
*  round-trips and still runs. What changes at this depth is only how it is
*  DRAWN — one `⟨advanced⟩` chip whose text is the subtree's phrase, edited in
*  the query text pane below.
*
*  Three, because the indented-rule rendering costs horizontal width per level
*  and the sheet has to survive a 360 px phone. And the bound covers EVERY
*  rendering of the tree, not only the rows: {@link filterPhrase} (and so
*  {@link filterLabel} and the chip's own text) stops here too, so a 64-deep
*  hostile query produces a short bounded sentence rather than a recursive dump
*  (I-22). Relation predicates and `off` count as levels exactly as rows do. */
export const MAX_QUERY_BUILDER_DEPTH = 3;

/** The filter shapes the add-picker can build and the edit popover can re-collect. */
export type BuilderLeafKind =
  | "page"
  | "task"
  | "priority"
  | "property"
  | "scheduled"
  | "deadline"
  | "journal"
  | "between"
  | "onPage"
  | "namespace"
  | "pageProperty"
  | "pageTags"
  | "content"
  | "search";

// Leaf constructors — the IR shapes `og.rs` builds for the same intent

const attr = (a: Attr, op: CmpOp, value: Value): Filter => ({
  kind: "leaf",
  leaf: { kind: "attr", attr: a, op, value },
});
const rel = (r: Rel, pred: Filter): Filter => ({
  kind: "leaf",
  leaf: { kind: "rel", rel: r, quant: "any", pred },
});
const textList = (items: string[]): Value => ({
  kind: "list",
  items: items.map((text) => ({ kind: "text", text }) as Value),
});
/** `og.rs::through_page`: a page-row test read through the block's owning page. */
const throughPage = (pred: Filter): Filter => rel("page", pred);

/** `og.rs::escape_like`. */
export function escapeLike(text: string): string {
  return text.replace(/[%_\\]/g, (ch) => `\\${ch}`);
}

/** `[[x]]` / `#x` — `og.rs` `page-ref` / `Filter::page_ref` (Q2). */
export function pageRefFilter(name: string): Filter {
  return rel("refs", attr("name", "eq", { kind: "text", text: name }));
}

/** `(task …)` — `og.rs` `"task" | "todo"`. */
export function taskFilter(markers: string[]): Filter {
  const picked = markers.length ? markers : ["TODO", "DOING", "NOW", "LATER"];
  return attr("task", "in", textList(picked));
}

/** `(priority …)` — `og.rs` `"priority"`. */
export function priorityFilter(levels: string[]): Filter {
  return attr("priority", "in", textList(levels.length ? levels : [...PRIORITIES]));
}

/** A typed comparison on a property's VALUE, chosen by {@link propertyOperators}
*  from the registry's effective type for the key. The plain-string form below is the
*  untyped `= '<text>'` this builder could always say; this is the durable half
*  P3 inherits, which is why the operator travels with its operand rather than
*  being reconstructed from the text at every call site. */
export interface PropertyValueTest {
  op: CmpOp;
  operand: Value;
}

/** `(property k)` / `(property k v)` — `og.rs::property_leaf`, the one shape §3.3 defines. */
export function propertyFilter(
  key: string,
  value: string | PropertyValueTest | null,
): Filter {
  const keyTest = attr("key", "eq", { kind: "text", text: key });
  const valueTest: Filter | null =
    value == null || value === ""
      ? null
      : typeof value === "string"
        ? attr("value", "eq", { kind: "text", text: value })
        : attr("value", value.op, value.operand);
  const pred: Filter = valueTest ? { kind: "and", items: [keyTest, valueTest] } : keyTest;
  return rel("props", pred);
}

/** `(page-property …)` — the same predicate read through the page. */
export function pagePropertyFilter(
  key: string,
  value: string | PropertyValueTest | null,
): Filter {
  return throughPage(propertyFilter(key, value));
}

// Typed operators (SPEC §7.4, §9 P2)

/** **A UI operator IDENTITY — not a `CmpOp` (SPEC §7.4, design §2.4).**
*
*  The row's `operator ▾` offers plain-English identities: *is more than*,
*  *does not contain*, *is not set*. Several of them are not operators at all
*  in the IR — *is not* is `not(prop('k') = v)`, *is blank* is a test on the
*  atom count, *is checked* is a fixed boolean — so an identity cannot be a
*  `CmpOp` and this union is deliberately its own vocabulary. Each identity has
*  an ENCODER ({@link encodePropertyLeaf}) and a DECODER
*  ({@link propertyLeafTest}), and the two are inverse; nothing here widens the
*  IR, because every encoding composes `CmpOp`s, `Value`s and `Rel` shapes P0
*  already has.
*
*  `has_other_value` is REOPEN-ONLY: it is P2's atom-level `!=`, which the menu
*  no longer offers because §3.3 makes it a different predicate from *is not*
*  (an owner WITHOUT the property matches *is not* and does not match `!=`). A
*  leaf that still carries it — reopened from P2 text, or typed in the pane —
*  must still reopen with its operator intact rather than be silently
*  rewritten, so it decodes to an identity with an honest label. */
export type PropertyOperatorId =
  | "is"
  | "is_not"
  | "gt"
  | "ge"
  | "lt"
  | "le"
  | "between"
  | "before"
  | "on_or_before"
  | "after"
  | "on_or_after"
  | "contains"
  | "does_not_contain"
  | "starts_with"
  | "ends_with"
  | "references"
  | "does_not_reference"
  | "is_checked"
  | "is_unchecked"
  | "is_set"
  | "is_not_set"
  | "is_blank"
  | "has_other_value";

/** One row of the `operator ▾` menu. */
export interface PropertyOperator {
  id: PropertyOperatorId;
  /** The words the row shows. Plain English, never the operator's spelling. */
  label: string;
  /** How many value inputs the row collects. */
  arity: 0 | 1 | 2;
}

const one = (texts: string[]): string => (texts[0] ?? "").trim();

const textOperand = (texts: string[]): Value | null => {
  const text = one(texts);
  return text ? { kind: "text", text } : null;
};

const numberOperand = (texts: string[]): Value | null => {
  const text = one(texts);
  if (!text) return null;
  const number = Number(text);
  return Number.isFinite(number) ? { kind: "number", number } : null;
};

/** §4.2.3/A6: a date operand is the LITERAL the author typed (`today`, `-30d`, `2026-01-01`, a journal title). */
const dateOperand = (texts: string[]): Value | null => {
  const literal = one(texts);
  return literal ? { kind: "date", literal } : null;
};

const boolOperand = (texts: string[]): Value | null => {
  const text = one(texts).toLowerCase();
  if (["true", "yes", "y", "1", "done", "checked"].includes(text)) {
    return { kind: "bool", bool: true };
  }
  if (["false", "no", "n", "0", "unchecked"].includes(text)) {
    return { kind: "bool", bool: false };
  }
  return null;
};

/** The scalar operand for a key of this effective type. */
function scalarOperand(texts: string[], type: ObservedType): Value | null {
  switch (type) {
    case "number":
      return numberOperand(texts);
    case "date":
      return dateOperand(texts);
    case "checkbox":
      return boolOperand(texts);
    default:
      return textOperand(texts);
  }
}

function rangeOperand(texts: string[], type: ObservedType): Value | null {
  const low = scalarOperand([texts[0] ?? ""], type);
  const high = scalarOperand([texts[1] ?? ""], type);
  return low && high ? { kind: "list", items: [low, high] } : null;
}

/** "contains" is a `like` PATTERN, not a substring: the user's `%`, `_` and `\`
*  are data (`escapeLike`), exactly as `contentFilter` builds it. */
const containsOperand = (texts: string[]): Value | null => {
  const text = one(texts);
  return text ? { kind: "text", text: `%${escapeLike(text)}%` } : null;
};

const endsWithOperand = (texts: string[]): Value | null => {
  const text = one(texts);
  return text ? { kind: "text", text: `%${escapeLike(text)}` } : null;
};

/** How one identity is spelled in the IR. */
interface Encoding {
  op: CmpOp;
  arity: 0 | 1 | 2;
  operand: (texts: string[], type: ObservedType) => Value | null;
  negate?: boolean;
}

const COMPARISONS: Partial<Record<PropertyOperatorId, Encoding>> = {
  is: { op: "eq", arity: 1, operand: scalarOperand },
  is_not: { op: "eq", arity: 1, operand: scalarOperand, negate: true },
  references: { op: "eq", arity: 1, operand: textOperand },
  does_not_reference: { op: "eq", arity: 1, operand: textOperand, negate: true },
  gt: { op: "gt", arity: 1, operand: scalarOperand },
  ge: { op: "ge", arity: 1, operand: scalarOperand },
  lt: { op: "lt", arity: 1, operand: scalarOperand },
  le: { op: "le", arity: 1, operand: scalarOperand },
  after: { op: "gt", arity: 1, operand: dateOperand },
  on_or_after: { op: "ge", arity: 1, operand: dateOperand },
  before: { op: "lt", arity: 1, operand: dateOperand },
  on_or_before: { op: "le", arity: 1, operand: dateOperand },
  between: { op: "between", arity: 2, operand: rangeOperand },
  contains: { op: "like", arity: 1, operand: containsOperand },
  does_not_contain: { op: "like", arity: 1, operand: containsOperand, negate: true },
  starts_with: { op: "starts_with", arity: 1, operand: textOperand },
  ends_with: { op: "like", arity: 1, operand: endsWithOperand },
  has_other_value: { op: "not_eq", arity: 1, operand: scalarOperand },
};

/** The identities with no value cell at all. */
const NULLARY: Partial<Record<PropertyOperatorId, true>> = {
  is_set: true,
  is_not_set: true,
  is_blank: true,
  is_checked: true,
  is_unchecked: true,
};

/** The words each identity shows. */
export function propertyOperatorLabel(
  id: PropertyOperatorId,
  cardinality: Cardinality = "one",
): string {
  const many = cardinality === "many";
  switch (id) {
    case "is":
      return many ? "contains this value" : "is";
    case "is_not":
      return many ? "does not contain this value" : "is not";
    case "gt":
      return "is more than";
    case "ge":
      return "is at least";
    case "lt":
      return "is less than";
    case "le":
      return "is at most";
    case "between":
      return "is between";
    case "before":
      return "before";
    case "on_or_before":
      return "on or before";
    case "after":
      return "after";
    case "on_or_after":
      return "on or after";
    case "contains":
      return "contains";
    case "does_not_contain":
      return "does not contain";
    case "starts_with":
      return "starts with";
    case "ends_with":
      return "ends with";
    case "references":
      return "references";
    case "does_not_reference":
      return "does not reference";
    case "is_checked":
      return "is checked";
    case "is_unchecked":
      return "is unchecked";
    case "is_set":
      return "is set";
    case "is_not_set":
      return "is not set";
    case "is_blank":
      return "is blank";
    case "has_other_value":
      return "has a value other than";
  }
}

/** The number of value inputs an identity collects. */
export function propertyOperatorArity(id: PropertyOperatorId): 0 | 1 | 2 {
  if (NULLARY[id]) return 0;
  return COMPARISONS[id]?.arity ?? 1;
}

const IDENTITIES: Record<ObservedType, PropertyOperatorId[]> = {
  number: ["is", "is_not", "gt", "ge", "lt", "le", "between", "is_set", "is_not_set", "is_blank"],
  // No "is not": a date atom has no honest negation short of `not(…)`, which the design table does not list …
  date: ["is", "before", "on_or_before", "after", "on_or_after", "between", "is_set", "is_not_set"],
  text: [
    "contains",
    "does_not_contain",
    "is",
    "is_not",
    "starts_with",
    "ends_with",
    "is_set",
    "is_not_set",
    "is_blank",
  ],
  ref: ["references", "does_not_reference"],
  checkbox: ["is_checked", "is_unchecked", "is_not_set"],
};

/**
* **The operator menu for a property key, from the registry's effective type
* (SPEC §7.4, design §2.4).**
*
* Pure, and the ONE producer of this question: the row renders exactly this
* list. Three rules it encodes:
*
*  - **It chooses among operators the IR already has.** `CmpOp`, `Value` and
*    the three §3.3 presence shapes are P0's; nothing here widens any of them.
*  - **`is set` / `is not set` / `is blank` are three different identities**,
*    because conflating them is a bug users report: absent, present-with-no-
*    value and present-with-any-value are three different questions, and the
*    registry knows which is which.
*  - **Every negative identity wraps the POSITIVE leaf in `not(…)`** (§3.3), so
*    an owner that lacks the property matches *is not*. The atom-level `!=` is
*    a different predicate and is no longer offered.
*
 * The design table's `enum` row (≤ N distinct values) is deliberately absent:
 * the registry has no enum type, so there is nothing to read it from.
 * Number offers comparisons and between; date offers before/after and between;
 * text offers substring and prefix/suffix checks. Ref offers references and its
 * negation; checkbox offers checked, unchecked and not set. `has_other_value`
 * is read back from IR but never offered in the menu.
*/
export function propertyOperators(effective: {
  type: ObservedType;
  cardinality: Cardinality;
}): PropertyOperator[] {
  return IDENTITIES[effective.type].map((id) => ({
    id,
    label: propertyOperatorLabel(id, effective.cardinality),
    arity: propertyOperatorArity(id),
  }));
}

const propsLeaf = (key: string, quant: Quant, atom: Filter | null): Filter => {
  const keyTest = attr("key", "eq", { kind: "text", text: key });
  return {
    kind: "leaf",
    leaf: {
      kind: "rel",
      rel: "props",
      quant,
      pred: atom ? { kind: "and", items: [keyTest, atom] } : keyTest,
    },
  };
};

/** What a row of the sheet holds: the identity, the key it tests, and the text in its value inputs. */
export interface PropertyLeafTest {
  id: PropertyOperatorId;
  key: string;
  /** One entry per value input, as typed. Empty for a nullary identity. */
  values: string[];
  /** A `page-property` row: the same predicate read through the owning page. */
  throughPage: boolean;
}

/**
* **An identity → the IR leaf it means.** The inverse of
* {@link propertyLeafTest} on every row of the §7.4 table.
*
* `null` when the text in the value inputs is not a value of this type — the
* row simply does not commit, rather than saving a leaf that matches nothing.
*/
export function encodePropertyLeaf(spec: {
  id: PropertyOperatorId;
  key: string;
  values?: string[];
  /** The key's effective type, which decides the operand KIND. */
  type?: ObservedType;
  throughPage?: boolean;
}): Filter | null {
  const { id, key, values = [], type = "text", throughPage: viaPage = false } = spec;
  if (!key) return null;
  const hop = (filter: Filter): Filter => (viaPage ? throughPage(filter) : filter);
  switch (id) {
    // The three §3.3 presence shapes, and nothing else may spell them.
    case "is_set":
      return hop(propsLeaf(key, "any", null));
    case "is_not_set":
      return hop(propsLeaf(key, "none", null));
    case "is_blank":
      return hop(propsLeaf(key, "any", attr("atom_count", "eq", { kind: "number", number: 0 })));
    case "is_checked":
      return hop(propsLeaf(key, "any", attr("value", "eq", { kind: "bool", bool: true })));
    case "is_unchecked":
      return hop(propsLeaf(key, "any", attr("value", "eq", { kind: "bool", bool: false })));
    default:
      break;
  }
  const encoding = COMPARISONS[id];
  if (!encoding) return null;
  const operand = encoding.operand(values, type);
  if (!operand) return null;
  const positive = hop(propsLeaf(key, "any", attr("value", encoding.op, operand)));
  return encoding.negate ? { kind: "not", inner: positive } : positive;
}

/** Every literal in a value, as the row's inputs hold it. */
function valueTexts(value: Value): string[] {
  switch (value.kind) {
    case "text":
      return [value.text];
    case "number":
      return [String(value.number)];
    case "date":
      return [value.literal];
    case "bool":
      return [String(value.bool)];
    case "list":
      return value.items.flatMap(valueTexts);
    case "none":
      return [];
  }
}

/** One token of a `like` pattern: a literal character or a wildcard. */
type LikeToken = { kind: "lit"; ch: string } | { kind: "any" } | { kind: "one" };

function likeTokens(pattern: string): LikeToken[] | null {
  const out: LikeToken[] = [];
  for (let i = 0; i < pattern.length; i++) {
    const ch = pattern[i];
    if (ch === "\\") {
      i += 1;
      if (i >= pattern.length) return null;
      out.push({ kind: "lit", ch: pattern[i] });
      continue;
    }
    if (ch === "%") out.push({ kind: "any" });
    else if (ch === "_") out.push({ kind: "one" });
    else out.push({ kind: "lit", ch });
  }
  return out;
}

/**
* **The inverse of {@link escapeLike}, told apart by SHAPE.**
*
* `%x%` is *contains*, `%x` is *ends with* and `x%` is *starts with* — three
* different identities, so reading a pattern back has to distinguish them
* rather than answer "some substring". The literal body is unescaped, so a
* value the user typed with `%`, `_` or `\` in it round-trips; an escaped
* wildcard is data, while an unescaped one in the middle is a pattern this
* builder did not write and has no identity for.
*/
export function readLikePattern(
  pattern: string,
): { shape: "contains" | "ends_with" | "starts_with" | "exact"; text: string } | null {
  const tokens = likeTokens(pattern);
  if (!tokens) return null;
  const lead = tokens[0]?.kind === "any";
  const trail = tokens.length > (lead ? 1 : 0) && tokens[tokens.length - 1].kind === "any";
  const body = tokens.slice(lead ? 1 : 0, trail ? tokens.length - 1 : tokens.length);
  if (body.some((token) => token.kind !== "lit")) return null;
  const text = body.map((token) => (token.kind === "lit" ? token.ch : "")).join("");
  if (!text) return null;
  if (lead && trail) return { shape: "contains", text };
  if (lead) return { shape: "ends_with", text };
  if (trail) return { shape: "starts_with", text };
  return { shape: "exact", text };
}

/**
* **An IR leaf → the row that edits it.** The inverse of
* {@link encodePropertyLeaf}, reading the WHOLE filter including a `not(…)`
* wrapper, so every property leaf the builder can construct reopens with its
* operator instead of silently losing it (the P2 follow-up this closes).
*
* `effective` disambiguates the ONE pair of identities with the same IR
* spelling: `prop('k') = 'x'` is *is* for a text key and *references* for a ref
* key. Everything else is told apart by the operator and the operand's kind.
*/
export function propertyLeafTest(
  filter: Filter,
  effective?: { type: ObservedType; cardinality?: Cardinality },
): PropertyLeafTest | null {
  let node = filter;
  let negated = false;
  if (node.kind === "not") {
    negated = true;
    node = node.inner;
  }
  let viaPage = false;
  const page = asRelLeaf(node, "page");
  if (page) {
    viaPage = true;
    node = page.pred;
  }
  const props = asRelLeaf(node, "props");
  if (!props) return null;
  const parts = propsParts(props.pred);
  if (!parts || parts.key === "tags") return null;
  const done = (id: PropertyOperatorId, values: string[] = []): PropertyLeafTest => ({
    id,
    key: parts.key,
    values,
    throughPage: viaPage,
  });
  if (!parts.atom) {
    if (negated) return null;
    if (props.quant === "any") return done("is_set");
    if (props.quant === "none") return done("is_not_set");
    return null;
  }
  if (props.quant !== "any") return null;
  const atom = asAttrLeaf(parts.atom);
  if (!atom) return null;
  if (atom.attr === "atom_count") {
    return !negated && atom.op === "eq" && atom.value.kind === "number" && atom.value.number === 0
      ? done("is_blank")
      : null;
  }
  if (atom.attr !== "value") return null;
  const texts = valueTexts(atom.value);
  const isRef = effective?.type === "ref";
  switch (atom.op) {
    case "eq":
      if (atom.value.kind === "bool") {
        return negated ? null : done(atom.value.bool ? "is_checked" : "is_unchecked");
      }
      if (atom.value.kind === "list") return null;
      if (isRef && atom.value.kind === "text") {
        return done(negated ? "does_not_reference" : "references", texts);
      }
      return done(negated ? "is_not" : "is", texts);
    case "not_eq":
      return negated ? null : done("has_other_value", texts);
    case "lt":
      return negated ? null : done(atom.value.kind === "date" ? "before" : "lt", texts);
    case "le":
      return negated ? null : done(atom.value.kind === "date" ? "on_or_before" : "le", texts);
    case "gt":
      return negated ? null : done(atom.value.kind === "date" ? "after" : "gt", texts);
    case "ge":
      return negated ? null : done(atom.value.kind === "date" ? "on_or_after" : "ge", texts);
    case "between":
      return negated || texts.length !== 2 ? null : done("between", texts);
    case "starts_with":
      return negated || atom.value.kind !== "text" ? null : done("starts_with", texts);
    case "like": {
      if (atom.value.kind !== "text") return null;
      const read = readLikePattern(atom.value.text);
      if (!read) return null;
      if (read.shape === "contains") {
        return done(negated ? "does_not_contain" : "contains", [read.text]);
      }
      if (read.shape === "ends_with") return negated ? null : done("ends_with", [read.text]);
      if (read.shape === "starts_with") return negated ? null : done("starts_with", [read.text]);
      return null;
    }
    default:
      return null;
  }
}

/** `(page-tags …)` — `og.rs` `"page-tags" | "tags"`: the page's `tags` property with a set test on its atoms. */
export function pageTagsFilter(tags: string[]): Filter {
  return throughPage(
    rel("props", {
      kind: "and",
      items: [attr("key", "eq", { kind: "text", text: "tags" }), attr("value", "in", textList(tags))],
    }),
  );
}

/** `(scheduled)` / `(deadline)` — presence, never OG-expressible (§3.3 B4). */
export function planningFilter(which: "scheduled" | "deadline"): Filter {
  return attr(which, "is_set", { kind: "none" });
}

/** `(journal)` — `og.rs` `"journal"`. */
export function journalFilter(): Filter {
  return throughPage(attr("journal", "eq", { kind: "bool", bool: true }));
}

/** `(page x)` — `og.rs` `"page"`. */
export function onPageFilter(name: string): Filter {
  return throughPage(attr("name", "eq", { kind: "text", text: name }));
}

/** `(namespace x)` — recursive membership: the normalized page name starts with
*  `x/` (§3.2 M20), `og.rs` `"namespace"`. */
export function namespaceFilter(ns: string): Filter {
  return throughPage(attr("name", "starts_with", { kind: "text", text: `${ns}/` }));
}

/** `og.rs::bounded`: two bounds are a `between`, one bound is the one-sided comparison, none is plain presence. */
function boundedFilter(which: Attr, start: string, end: string): Filter {
  const low = start.trim();
  const high = end.trim();
  if (low && high) {
    return attr(which, "between", {
      kind: "list",
      items: [
        { kind: "date", literal: low },
        { kind: "date", literal: high },
      ],
    });
  }
  if (low) return attr(which, "ge", { kind: "date", literal: low });
  if (high) return attr(which, "le", { kind: "date", literal: high });
  return attr(which, "is_set", { kind: "none" });
}

/** `(between [field] start end)` — `og.rs::between`. */
export function betweenFilter(field: BetweenField, start: string, end: string): Filter {
  switch (field) {
    case "journal":
      return throughPage(boundedFilter("day", start, end));
    case "scheduled":
      return boundedFilter("scheduled", start, end);
    case "deadline":
      return boundedFilter("deadline", start, end);
    case "any":
      return {
        kind: "or",
        items: [
          throughPage(boundedFilter("day", start, end)),
          boundedFilter("scheduled", start, end),
          boundedFilter("deadline", start, end),
        ],
      };
  }
}

/** A bare quoted string — `og.rs::content_like`: a case-insensitive substring
*  test on the block's visible content. */
export function contentFilter(text: string): Filter {
  return attr("content", "like", { kind: "text", text: `%${escapeLike(text)}%` });
}

/** `(search "…")` — the friendly-search grammar, `og.rs` `"search"`. */
export function searchFilter(source: string): Filter {
  return attr("content", "match", { kind: "text", text: source });
}

// Recognizers — which builder shape, if any, an IR node is

function asAttrLeaf(filter: Filter): (Leaf & { kind: "attr" }) | null {
  return filter.kind === "leaf" && filter.leaf.kind === "attr" ? filter.leaf : null;
}
function asRelLeaf(filter: Filter, which: Rel): (Leaf & { kind: "rel" }) | null {
  return filter.kind === "leaf" && filter.leaf.kind === "rel" && filter.leaf.rel === which
    ? filter.leaf
    : null;
}
function textOf(value: Value): string | null {
  return value.kind === "text" ? value.text : null;
}
function listOf(value: Value): string[] | null {
  if (value.kind !== "list") return null;
  const out: string[] = [];
  for (const item of value.items) {
    const text = textOf(item);
    if (text == null) return null;
    out.push(text);
  }
  return out;
}
function dateOf(value: Value): string | null {
  return value.kind === "date" ? value.literal : null;
}

/** The `props` predicate's key and optional single atom test, mirroring
*  `Filter::props_key` / `props_atom_test` on the Rust side. */
function propsParts(pred: Filter): { key: string; atom: Filter | null } | null {
  const items = pred.kind === "and" ? pred.items : [pred];
  let key: string | null = null;
  let atom: Filter | null = null;
  for (const item of items) {
    const leaf = asAttrLeaf(item);
    if (leaf && leaf.attr === "key" && leaf.op === "eq") {
      const text = textOf(leaf.value);
      if (text == null || key != null) return null;
      key = text;
      continue;
    }
    if (atom != null) return null;
    atom = item;
  }
  return key == null ? null : { key, atom };
}

/** The property key a `props`/`page_prop` leaf tests, or `null` for any other shape. */
export function propertyLeafKey(filter: Filter): string | null {
  const page = asRelLeaf(filter, "page");
  if (page) return propertyLeafKey(page.pred);
  const props = asRelLeaf(filter, "props");
  if (!props) return null;
  const parts = propsParts(props.pred);
  return parts && parts.key !== "tags" ? parts.key : null;
}

/** Which builder shape this filter is, or `null` for anything the pickers cannot re-collect. */
export function builderLeafKind(filter: Filter): BuilderLeafKind | null {
  const page = asRelLeaf(filter, "page");
  if (page) {
    const inner = builderLeafKind(page.pred);
    if (inner === "property") return "pageProperty";
    if (inner === "pageTags") return "pageTags";
    const leaf = asAttrLeaf(page.pred);
    if (leaf?.attr === "journal") return "journal";
    if (leaf?.attr === "name" && leaf.op === "eq") return "onPage";
    if (leaf?.attr === "name" && leaf.op === "starts_with") return "namespace";
    if (leaf?.attr === "day") return "between";
    return null;
  }
  const refs = asRelLeaf(filter, "refs");
  if (refs) {
    const leaf = asAttrLeaf(refs.pred);
    return leaf?.attr === "name" && leaf.op === "eq" ? "page" : null;
  }
  const props = asRelLeaf(filter, "props");
  if (props) {
    const parts = propsParts(props.pred);
    if (!parts) return null;
    const atom = parts.atom ? asAttrLeaf(parts.atom) : null;
    if (parts.key === "tags" && atom?.attr === "value" && atom.op === "in") return "pageTags";
    if (!parts.atom) return "property";
    return atom?.attr === "value" && atom.op === "eq" ? "property" : null;
  }
  const leaf = asAttrLeaf(filter);
  if (!leaf) return null;
  switch (leaf.attr) {
    case "task":
      return leaf.op === "in" ? "task" : null;
    case "priority":
      return leaf.op === "in" ? "priority" : null;
    case "scheduled":
    case "deadline":
      if (leaf.op === "is_set") return leaf.attr === "scheduled" ? "scheduled" : "deadline";
      return leaf.op === "between" || leaf.op === "ge" || leaf.op === "le" ? "between" : null;
    case "content":
      if (leaf.op === "like") return "content";
      return leaf.op === "match" ? "search" : null;
    default:
      return null;
  }
}

// Human-readable chip labels

const ATTR_PHRASE: Record<Attr, string> = {
  content: "text",
  task: "task",
  priority: "priority",
  scheduled: "scheduled",
  deadline: "deadline",
  name: "name",
  journal: "journal",
  day: "date",
  namespace: "namespace",
  key: "key",
  value: "value",
  atom_count: "values",
};
const OP_PHRASE: Record<CmpOp, string> = {
  eq: "is",
  not_eq: "is not",
  lt: "<",
  le: "≤",
  gt: ">",
  ge: "≥",
  between: "between",
  in: "is one of",
  not_in: "is none of",
  like: "contains",
  starts_with: "starts with",
  match: "matches",
  regex: "matches regex",
  is_set: "is set",
  is_not_set: "is not set",
  is_blank: "is blank",
};

function valuePhrase(value: Value): string {
  switch (value.kind) {
    case "text":
      return value.text;
    case "number":
      return String(value.number);
    case "date":
      return value.literal;
    case "bool":
      return value.bool ? "yes" : "no";
    case "list":
      return value.items.map(valuePhrase).join(" | ");
    case "none":
      return "";
  }
}

function betweenPhrase(value: Value): string {
  if (value.kind !== "list" || value.items.length !== 2) return valuePhrase(value);
  return `${dateOf(value.items[0]) ?? valuePhrase(value.items[0])} ~ ${dateOf(value.items[1]) ?? valuePhrase(value.items[1])}`;
}

/** One piece of a rendered phrase (SPEC §7.2). */
export interface PhraseSegment {
  kind: "text" | "value" | "field" | "advanced";
  text: string;
  title?: string;
}

const seg = (kind: PhraseSegment["kind"], text: string, title?: string): PhraseSegment =>
  title ? { kind, text, title } : { kind, text };
const words = (text: string): PhraseSegment => seg("text", text);
const chip = (text: string, title?: string): PhraseSegment => seg("value", text, title);
const named = (text: string): PhraseSegment => seg("field", text);

/** The one bounded segment a subtree past {@link MAX_QUERY_BUILDER_DEPTH} collapses to. */
export const ADVANCED_PHRASE = "⟨advanced⟩";

/** **The phrase for one filter node — the ONE producer (§7.2).**
*
*  `filterLabel` is exactly this, joined; the sentence, the row and the
*  `⟨advanced⟩` chip all read the same segments. Splitting the phrase into
*  segments rather than a string is what lets the sentence draw values as chips
*  without a second phrasing function that would drift from this one (I-12).
*
*  **Total by construction (the anti-Jira property, §7.5):** every query the
*  parser accepts renders in the builder, invalid ones included — so this never
*  returns "unsupported" and never throws. **Bounded by construction (I-22):**
*  relation predicates and `off` are levels, and past the cap the answer is one
*  `advanced` segment. */
export function filterPhrase(filter: Filter, depth = 0): PhraseSegment[] {
  if (depth >= MAX_QUERY_BUILDER_DEPTH) return [seg("advanced", ADVANCED_PHRASE)];
  switch (filter.kind) {
    case "and":
    case "or":
      return [words(filter.kind.toUpperCase())];
    case "not":
      return [words("NOT")];
    case "off":
      return [...filterPhrase(filter.inner, depth + 1), words(" (off)")];
    case "raw":
      return [chip(filter.text)];
    case "true":
      return [words("everything")];
    case "false":
      return [words("nothing")];
    case "leaf":
      return leafPhrase(filter, filter.leaf, depth);
  }
}

/** The phrase for one filter node, as a plain string. */
export function filterLabel(filter: Filter): string {
  return filterPhrase(filter)
    .map((segment) => segment.text)
    .join("");
}

/** **What the row's VALUE cell says: the operands, without the prose.**
*
*  A row already names its field and its operator in its own two cells, so the
*  value cell must not repeat them — `Task marker ▾ | is any of ▾ | task: TODO
*  | DOING` says "task" three times. This is not a second labeller (D-14): it
*  is the SAME {@link filterPhrase} segments with the prose dropped, so a value
*  can never drift from the sentence that quotes it. A leaf with no operands at
*  all (`on journal page`) keeps its whole phrase, because an empty cell would
*  be nothing to click. */
export function filterValueLabel(filter: Filter): string {
  const segments = filterPhrase(filter);
  const values = segments.filter((segment) => segment.kind === "value");
  return values.length ? values.map((segment) => segment.text).join(" ") : filterLabel(filter);
}

function leafPhrase(filter: Filter, leaf: Leaf, depth: number): PhraseSegment[] {
  const kind = builderLeafKind(filter);
  if (leaf.kind === "rel") {
    const inner = asAttrLeaf(leaf.pred);
    const innerText = inner ? (textOf(inner.value) ?? "") : "";
    switch (kind) {
      case "page":
        return [chip(innerText)];
      case "onPage":
        return [words("page: "), chip(innerText)];
      case "namespace":
        return [words("namespace: "), chip(innerText.replace(/\/$/, ""))];
      case "journal":
        return [words("on journal page")];
      default:
        break;
    }
    if (leaf.rel === "page") {
      const inner = filterPhrase(leaf.pred, depth + 1);
      return kind === "pageProperty" ? [words("page "), ...inner] : inner;
    }
    if (leaf.rel === "props") {
      const parts = propsParts(leaf.pred);
      if (parts) {
        const atom = parts.atom ? asAttrLeaf(parts.atom) : null;
        if (parts.key === "tags" && atom?.op === "in") {
          return [words("page tags: "), chip((listOf(atom.value) ?? []).join(" | "))];
        }
        if (!parts.atom) return [named(parts.key), words(": any")];
        if (atom?.op === "eq") {
          return [named(parts.key), words(": "), chip(valuePhrase(atom.value))];
        }
      }
      // A typed comparison.
      const test = propertyLeafTest(filter);
      if (test) return propertyTestPhrase(test);
    }
    return [words(`${leaf.quant} ${leaf.rel}: `), ...filterPhrase(leaf.pred, depth + 1)];
  }
  // An attribute leaf.
  switch (kind) {
    case "task":
      return [words("task: "), chip((listOf(leaf.value) ?? []).join(" | ") || "any")];
    case "priority":
      return [words("priority: "), chip((listOf(leaf.value) ?? []).join(" | ") || "any")];
    case "scheduled":
      return [words("scheduled")];
    case "deadline":
      return [words("deadline")];
    case "content":
      return [
        words('text: "'),
        chip(plainLikeSubstring(textOf(leaf.value) ?? "") ?? valuePhrase(leaf.value)),
        words('"'),
      ];
    case "search":
      return [words("search: "), chip(valuePhrase(leaf.value))];
    default:
      break;
  }
  if (leaf.op === "between") {
    const field = leaf.attr === "day" ? "" : `${ATTR_PHRASE[leaf.attr]} `;
    return [words(`${field}between: `), chip(betweenPhrase(leaf.value))];
  }
  if (leaf.op === "is_set" || leaf.op === "is_not_set" || leaf.op === "is_blank") {
    return [named(ATTR_PHRASE[leaf.attr]), words(` ${OP_PHRASE[leaf.op]}`)];
  }
  return [
    named(ATTR_PHRASE[leaf.attr]),
    words(` ${OP_PHRASE[leaf.op]} `),
    chip(valuePhrase(leaf.value)),
  ];
}

/** `cost is more than 100` — the row's own words, in the sentence. */
function propertyTestPhrase(test: PropertyLeafTest): PhraseSegment[] {
  const label = propertyOperatorLabel(test.id);
  if (!test.values.length) return [named(test.key), words(` ${label}`)];
  return [named(test.key), words(` ${label} `), chip(test.values.join(" ~ "))];
}

/** Whether a node is a single condition rather than a group — the unit that
*  renders as ONE row, and the unit a `not`/`off` wrapper can decorate without
*  costing a level of indentation. */
function isLeafLike(filter: Filter): boolean {
  return (
    filter.kind === "leaf" ||
    filter.kind === "raw" ||
    filter.kind === "true" ||
    filter.kind === "false"
  );
}

/** Whether the filter places no condition: true or an empty and. An empty or
 *  is false in the engine and matches nothing. */
export function isEmptyFilter(filter: Filter): boolean {
  if (filter.kind === "true") return true;
  return filter.kind === "and" && filter.items.length === 0;
}

function joinPhrases(parts: PhraseSegment[][], connector: "and" | "or"): PhraseSegment[] {
  if (parts.length === 0) return [];
  if (parts.length === 1) return parts[0];
  const out: PhraseSegment[] = [];
  parts.forEach((part, index) => {
    if (index > 0) {
      const last = index === parts.length - 1;
      // A list reads as a list: "a, b, and c" — commas between, the connector once, before the last.
      if (connector === "and") out.push(words(last ? (parts.length > 2 ? ", and " : " and ") : ", "));
      else out.push(words(last ? " or " : ", "));
    }
    out.push(...part);
  });
  return out;
}

function clausePhrase(filter: Filter, depth: number, isRoot: boolean): PhraseSegment[] {
  if (depth >= MAX_QUERY_BUILDER_DEPTH) return [seg("advanced", ADVANCED_PHRASE)];
  switch (filter.kind) {
    case "and":
    case "or": {
      if (filter.items.length === 0) return filterPhrase(filter, depth);
      // A one-item group needs no parentheses, but it is still a LEVEL: a chain of them is exactly what a hostile …
      if (filter.items.length === 1) return clausePhrase(filter.items[0], depth + 1, isRoot);
      const parts = filter.items.map((item) => clausePhrase(item, depth + 1, false));
      const joined = joinPhrases(parts, filter.kind === "or" ? "or" : "and");
      return isRoot ? joined : [words("("), ...joined, words(")")];
    }
    case "not": {
      const inner = isLeafLike(filter.inner)
        ? clausePhrase(filter.inner, depth, false)
        : clausePhrase(filter.inner, depth + 1, false);
      return [words("not "), ...inner];
    }
    case "off": {
      const inner = isLeafLike(filter.inner)
        ? clausePhrase(filter.inner, depth, false)
        : clausePhrase(filter.inner, depth + 1, false);
      return [...inner, words(" (off)")];
    }
    default:
      return filterPhrase(filter, depth);
  }
}

/**
* **The resting sentence for a whole query (SPEC §7.2, design §2.1).**
*
* One plain-English line whose subject is the anchor — "Blocks where …" /
* "Pages where …" — and whose predicate is the filter read as prose, values as
* soft chips. It says "where" because the sheet's anchor line says "Find
* blocks where …": the resting line and the editing line are one sentence.
* An empty filter reads "All blocks" / "All pages": the honest
* answer, and not a menu.
*
* It composes {@link filterPhrase} rather than re-phrasing anything, so the row
* a user edits and the sentence they read are the same words, and it is bounded
* by the same cap (I-22).
*/
export function querySentence(query: { anchor: Anchor; filter: Filter }): PhraseSegment[] {
  const plural = query.anchor === "page" ? "pages" : "blocks";
  if (isEmptyFilter(query.filter)) return [words("All "), named(plural)];
  const subject = plural === "pages" ? "Pages" : "Blocks";
  return [named(subject), words(" where "), ...clausePhrase(query.filter, 0, true)];
}

/** The inverse of {@link escapeLike} for a pattern that is exactly `%<literal>%`
*  — `og::plain_like_substring`, so a `contains` chip shows the words the user
*  typed rather than the pattern the engine runs. */
export function plainLikeSubstring(pattern: string): string | null {
  if (!pattern.startsWith("%") || !pattern.endsWith("%") || pattern.length < 2) return null;
  const inner = pattern.slice(1, -1);
  let out = "";
  for (let i = 0; i < inner.length; i++) {
    const ch = inner[i];
    if (ch === "\\") {
      i += 1;
      if (i >= inner.length) return null;
      out += inner[i];
      continue;
    }
    if (ch === "%" || ch === "_") return null;
    out += ch;
  }
  return out;
}

export { SORT_PRESETS, sortLabel, currentSort, withSort, currentAgg, withAgg, currentGroup, withGroup } from "./queryViewSettings";
export type { SortPreset, AggState } from "./queryViewSettings";

// Immutable tree edits.

/** The child list of a boolean node, or `null` for a leaf/raw/true/false. */
export function filterChildren(filter: Filter): Filter[] | null {
  switch (filter.kind) {
    case "and":
    case "or":
      return filter.items;
    case "not":
    case "off":
      return [filter.inner];
    default:
      return null;
  }
}

function withChildren(filter: Filter, children: Filter[]): Filter {
  switch (filter.kind) {
    case "and":
      return { kind: "and", items: children };
    case "or":
      return { kind: "or", items: children };
    case "not":
      return children[0] ? { kind: "not", inner: children[0] } : { kind: "and", items: [] };
    case "off":
      return children[0] ? { kind: "off", inner: children[0] } : { kind: "and", items: [] };
    default:
      return filter;
  }
}

/** The root the bar edits: always an `and`/`or` node, so "add a filter here" has somewhere to add. */
export function builderRoot(filter: Filter): Filter {
  if (filter.kind === "and" || filter.kind === "or") return filter;
  if (filter.kind === "true") return { kind: "and", items: [] };
  return { kind: "and", items: [filter] };
}

function clone(filter: Filter): Filter {
  return structuredClone(filter);
}

/** Resolve `loc` to the node that CONTAINS the addressed child, plus the index within it. */
function locate(root: Filter, loc: number[]): { parent: Filter; children: Filter[]; idx: number } | null {
  if (loc.length === 0) return null;
  let node = root;
  for (let i = 0; i < loc.length - 1; i++) {
    const kids = filterChildren(node);
    if (!kids) return null;
    const next = kids[loc[i]];
    if (!next) return null;
    node = next;
  }
  const children = filterChildren(node);
  if (!children) return null;
  return { parent: node, children, idx: loc[loc.length - 1] };
}

/** Resolve `loc` to the node it addresses (`[]` = root). */
function nodeAt(root: Filter, loc: number[]): Filter | null {
  let node = root;
  for (const i of loc) {
    const kids = filterChildren(node);
    if (!kids) return null;
    const next = kids[i];
    if (!next) return null;
    node = next;
  }
  return node;
}

/** Drop empty `and`/`or` nodes anywhere except the root, so an edit never leaves
*  a vacuous `and([])` behind that would match everything. */
function prune(filter: Filter): Filter | null {
  const children = filterChildren(filter);
  if (!children) return filter;
  if (filter.kind === "not" || filter.kind === "off") {
    const inner = prune(children[0]);
    return inner ? withChildren(filter, [inner]) : null;
  }
  const kids = children.map(prune).filter((x): x is Filter => x != null);
  if (kids.length === 0) return null;
  return withChildren(filter, kids);
}

function normalize(root: Filter): Filter {
  const children = filterChildren(root);
  if (!children) return root;
  const kids = children.map(prune).filter((x): x is Filter => x != null);
  // A `not`/`off` root has no enclosing position, so it becomes an `and` root holding what survived — the same …
  if (root.kind === "not" || root.kind === "off") return { kind: "and", items: kids };
  return withChildren(root, kids);
}

/** **Put `next` where `loc` points, whatever kind of node holds that place.**
*
*  `locate` above answers a LIST question — it hands back the child array a
*  splice needs — and `filterChildren` synthesizes a fresh one-element array for
*  the unary `not`/`off`. That is right for a splice (you cannot splice two
*  children into a `not`) and silently wrong for an assignment: writing into the
*  synthesized array wrote into a copy, so every edit addressed at a child of a
*  unary wrapper did nothing and STILL returned a new tree, which the sheet
*  saved. The three group actions that address the `and`/`or` inside its
*  wrapper — the all/any header, "None of" and "Ungroup" — were therefore dead
*  clicks that wrote the block and pushed an empty step onto the undo stack, on
*  every `none of` group and (since P6 let a group be switched off) on every
*  disabled one. Assignment goes through here instead. */
function assignAt(draft: Filter, loc: number[], next: Filter): boolean {
  if (loc.length === 0) return false;
  let node = draft;
  for (let i = 0; i < loc.length - 1; i++) {
    const kids = filterChildren(node);
    const child = kids?.[loc[i]];
    if (!child) return false;
    node = child;
  }
  const index = loc[loc.length - 1];
  if (node.kind === "and" || node.kind === "or") {
    if (!node.items[index]) return false;
    node.items[index] = next;
    return true;
  }
  if (node.kind === "not" || node.kind === "off") {
    if (index !== 0) return false;
    node.inner = next;
    return true;
  }
  return false;
}

/** Mutating a node the path does not address is a no-op that returns the input
*  unchanged, so a stale `loc` from a popover that outlived its tree cannot
*  corrupt the query. */
function edit(root: Filter, apply: (draft: Filter) => boolean): Filter {
  const draft = clone(root);
  return apply(draft) ? normalize(draft) : root;
}

/** Append `filter` to the boolean node addressed by `opLoc` (`[]` = root). */
export function addChild(root: Filter, opLoc: number[], filter: Filter): Filter {
  return edit(root, (draft) => {
    const node = nodeAt(draft, opLoc);
    const children = node ? filterChildren(node) : null;
    if (!node || !children || node.kind === "not" || node.kind === "off") return false;
    children.push(filter);
    return true;
  });
}

export function removeAt(root: Filter, loc: number[]): Filter {
  return edit(root, (draft) => {
    const at = locate(draft, loc);
    if (!at || !at.children[at.idx]) return false;
    at.children.splice(at.idx, 1);
    return true;
  });
}

export function replaceAt(root: Filter, loc: number[], filter: Filter): Filter {
  return edit(root, (draft) => assignAt(draft, loc, filter));
}

/** Wrap the node at `loc` in a new boolean node. */
export function wrapAt(root: Filter, loc: number[], op: "and" | "or" | "not" | "off"): Filter {
  return edit(root, (draft) => {
    const current = nodeAt(draft, loc);
    if (!current) return false;
    return assignAt(
      draft,
      loc,
      op === "not"
        ? { kind: "not", inner: current }
        : op === "off"
          ? { kind: "off", inner: current }
          : { kind: op, items: [current] },
    );
  });
}

// Grouping, reordering and disabling (SPEC §7.4 remainder, P6)

/** The three group headers the sheet offers, in the sheet's own words. */
export type GroupChoice = "all" | "any" | "none";

const groupNode = (choice: GroupChoice, items: Filter[]): Filter =>
  choice === "any"
    ? { kind: "or", items }
    : choice === "none"
      ? { kind: "not", inner: { kind: "or", items } }
      : { kind: "and", items };

/** **Group the selected siblings into one group (§7.4, design §2.5).**
*
*  The ONE grouping operation: multi-select grouping, "group with the row
*  above" and any future gesture all come through here, so the answer to "which
*  rows end up where" is written once.
*
*  Three rules, and each of them is a way a selection can be quietly betrayed:
*
*   - **Selected items keep their original relative order.** Grouping is not a
*     sort, and the order of an `and`/`or` list is the order the author typed
*     (§3.5 keeps child order in the editable form).
*   - **The group is inserted at the FIRST selected position**, and the
*     unselected siblings keep their own order around it. A non-contiguous
*     selection follows the same rule rather than a second one: the group lands
*     where the topmost selected row was.
*   - **Anything that is not a set of siblings is REFUSED**, not repaired. Locs
*     from two different lists, a loc and its own descendant, a duplicate, an
*     index past the end, a stale path from a menu that outlived its tree —
*     each returns the tree unchanged, exactly as every other edit here does.
*     Sibling-ness is what makes ancestor/descendant selection impossible: two
*     locs with the same parent path can never nest.
*
*  Fewer than two locs is a refusal too: "group" of one row is a wrapper the
*  user did not ask for. */
export function groupSelected(root: Filter, locs: number[][], choice: GroupChoice): Filter {
  if (locs.length < 2) return root;
  const parent = locs[0].slice(0, -1);
  const sibling = (loc: number[]) =>
    loc.length === parent.length + 1 && parent.every((step, i) => loc[i] === step);
  if (!locs.every(sibling)) return root;
  const indices = [...new Set(locs.map((loc) => loc[loc.length - 1]))].sort((a, b) => a - b);
  if (indices.length !== locs.length) return root;
  return edit(root, (draft) => {
    const node = nodeAt(draft, parent);
    if (!node || (node.kind !== "and" && node.kind !== "or")) return false;
    const children = node.items;
    if (indices.some((index) => !Number.isInteger(index) || index < 0 || index >= children.length)) {
      return false;
    }
    const picked = indices.map((index) => children[index]);
    const chosen = new Set(indices);
    const kept = children.filter((_, index) => !chosen.has(index));
    // Where the group goes among what is LEFT: as many unselected siblings precede it as preceded the first …
    const before = children.slice(0, indices[0]).filter((_, index) => !chosen.has(index)).length;
    kept.splice(before, 0, groupNode(choice, picked));
    node.items = kept;
    return true;
  });
}

/** **"Group with the row above" (§7.4, design §2.5).**
*
*  The addressed row and the sibling immediately before it, in place. It is
*  {@link groupSelected} with the selection the menu implies rather than a
*  second implementation of the same question — which is why it offers the same
*  all/any/none choices and lands in the same place. The first row of a list has
*  nothing above it, so it is a no-op, as is a stale `loc`. */
export function groupWithPrevious(root: Filter, loc: number[], choice: GroupChoice = "all"): Filter {
  if (loc.length === 0) return root;
  const previous = [...loc.slice(0, -1), loc[loc.length - 1] - 1];
  return groupSelected(root, [previous, loc], choice);
}

/** **Move a row or group among its own siblings (§7.4, P6).**
*
*  `to` is the index the node ends up at in the SAME list — the drop position a
*  drag reports and the one step "move up"/"move down" ask for, which is why
*  keyboard and pointer produce the same tree.
*
*  **Atomic against the original tree.** Removing the node and inserting it
*  again are one edit over one draft, so the destination is computed against the
*  list the user was looking at. A remove-then-insert built out of `removeAt`
*  and `addChild` would normalize in between: `removeAt` PRUNES an `and`/`or`
*  its last child just left, so moving the only row out of a group would delete
*  the group and shift every path after it — including the one holding the
*  destination. Boundary destinations are refused rather than clamped: a clamp
*  turns "I pressed up on the first row" into a silent save of an unchanged
*  tree. */
export function moveSibling(root: Filter, loc: number[], to: number): Filter {
  return edit(root, (draft) => {
    const at = locate(draft, loc);
    if (!at) return false;
    const node = at.children[at.idx];
    if (!node) return false;
    if (!Number.isInteger(to) || to < 0 || to >= at.children.length || to === at.idx) return false;
    at.children.splice(at.idx, 1);
    at.children.splice(to, 0, node);
    return true;
  });
}

/** The path of the node's OWN `off` wrapper, relative to the node, or `null` when it has none. */
function ownOffPath(node: Filter): number[] | null {
  if (node.kind === "off") return [];
  if (node.kind === "not" && node.inner.kind === "off") return [0];
  return null;
}

/** Whether the node at `loc` carries its OWN `off` wrapper. */
export function isDisabledAt(root: Filter, loc: number[]): boolean {
  const node = nodeAt(root, loc);
  return !!node && ownOffPath(node) !== null;
}

/** **The enabled control: add or remove this node's own `Off` (§3.5, §7.4).**
*
*  Disabling WRAPS the addressed node whole, so the `not` that spells the row's
*  negative operator, the `raw` payload of a condition the parser could not
*  read, an opaque advanced subtree past the rendering cap, and any `off` a
*  DESCENDANT carries all travel inside it untouched. Enabling removes exactly
*  the one wrapper the row draws as its greyed state and nothing else, so a
*  disabled group full of individually disabled rows comes back as it went in.
*
*  Nothing is deleted, coerced or re-read: `Off` is structural omission, and a
*  re-enabled `Raw` is the same bytes with the same diagnostic it always had
*  (§4.3.2). */
export function toggleDisabledAt(root: Filter, loc: number[]): Filter {
  if (loc.length === 0) return root;
  const node = nodeAt(root, loc);
  if (!node) return root;
  const off = ownOffPath(node);
  if (off === null) return wrapAt(root, loc, "off");
  // Enabling REBUILDS the row's whole node rather than addressing the `off` inside it.
  const inner = off.length === 0
    ? (node as Filter & { kind: "off" }).inner
    : { kind: "not" as const, inner: ((node as Filter & { kind: "not" }).inner as Filter & { kind: "off" }).inner };
  return replaceAt(root, loc, clone(inner));
}

/** Replace the boolean node at `loc` with its children spliced into the parent. */
export function unwrapAt(root: Filter, loc: number[]): Filter {
  const current = nodeAt(root, loc);
  const kids = current ? filterChildren(current) : null;
  if (!current || !kids || kids.length === 0) return root;
  const parent = loc.length > 1 ? nodeAt(root, loc.slice(0, -1)) : root;
  if (parent && (parent.kind === "not" || parent.kind === "off")) {
    return kids.length === 1 ? replaceAt(root, loc, clone(kids[0])) : root;
  }
  return edit(root, (draft) => {
    const at = locate(draft, loc);
    if (!at || !at.children[at.idx]) return false;
    at.children.splice(at.idx, 1, ...(filterChildren(at.children[at.idx]) ?? []));
    return true;
  });
}

/** Change `and` ↔ `or` on the node addressed by `loc` (`[]` = root). */
export function setOp(root: Filter, loc: number[], op: "and" | "or"): Filter {
  if (loc.length === 0) {
    if (root.kind !== "and" && root.kind !== "or") return root;
    return normalize({ kind: op, items: structuredClone(filterChildren(root) ?? []) });
  }
  return edit(root, (draft) => {
    const current = nodeAt(draft, loc);
    if (!current || (current.kind !== "and" && current.kind !== "or")) return false;
    return assignAt(draft, loc, { kind: op, items: current.items });
  });
}
