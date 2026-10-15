import { isLeafLike } from "./queryIr";
import { msg, render, t, type MessageId, type Msg } from "../i18n";
import type { MessageArgs } from "../i18n/messages.gen";
// The visual builder's model — **over the IR, not over text** (SPEC §7.1, §7.4).

import { MARKERS as TASK_MARKERS } from "../markers";
import { DATE_PRESETS } from "./dateExpr";
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
*  `QUERY_NESTING_MAX` is 128 and a deeper query still parses, still
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

/** `(task …)` — `og.rs` `"task" | "todo"`: the markers as given. An empty list
*  is the IR of a bare `(task)`, which in a Logseq query adds no condition
*  (D-18, GH #422; `Filter::is_dropped_clause`). The builder never writes it:
*  the marker pick's Apply needs at least one marker, and "Any status" is the
*  explicit list {@link anyTaskFilter}. */
export function taskFilter(markers: string[]): Filter {
  return attr("task", "in", textList(markers));
}

/** Every task status OG knows: OG `logseq.db.default/built-in-markers`
*  (`deps/db/src/logseq/db/default.cljs`). OG's `(task …)` rule needs a
*  non-empty marker set, so "Any status" is spelled as this list — the one
*  spelling OG reads as "every task". Tine's own marker list also has `STARTED`,
*  which OG does not know; it is deliberately NOT written, so the text stays
*  exactly what OG would run (GH #619, item 2). */
export const OG_TASK_MARKERS: readonly string[] = [
  "NOW",
  "LATER",
  "DOING",
  "DONE",
  "CANCELED",
  "CANCELLED",
  "IN-PROGRESS",
  "TODO",
  "WAIT",
  "WAITING",
];

/** The "Any status" condition: `(task NOW LATER …)` over every OG marker. */
export function anyTaskFilter(): Filter {
  return taskFilter([...OG_TASK_MARKERS]);
}

/** Whether a marker list says "Any status": exactly OG's set, or every marker
*  Tine itself knows (a hand-written list covering both reads the same). */
export function isAnyTaskStatus(markers: string[]): boolean {
  const have = new Set(markers.map((m) => m.toUpperCase()));
  const covers = (all: readonly string[]) =>
    have.size === all.length && all.every((m) => have.has(m));
  return covers(OG_TASK_MARKERS) || covers(TASK_MARKERS);
}

/** `(priority …)` — `og.rs` `"priority"`: the levels as given. An empty list is
*  a bare `(priority)`, which adds no condition (D-18); the builder's pick
*  needs at least one level, so it never writes one. */
export function priorityFilter(levels: string[]): Filter {
  return attr("priority", "in", textList(levels));
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
  return t("query-prose-property-operator", { op: id, cardinality: cardinality === "many" ? "list" : "scalar" });
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

/** The range "In a journal page" writes: wider than any journal date, in OG's
*  own relative-date spelling (`->journal-day-int` accepts a signed count and a
*  `y` unit; `(t/years -2000)` is year 26, `(t/years 2000)` year 4026). */
export const JOURNAL_ANY_RANGE: readonly [string, string] = ["-2000y", "+2000y"];

/** "In a journal page" — `(between -2000y +2000y)`. OG has no `(journal)`
*  filter; its `between` rule (`deps/db/src/logseq/db/rules.cljc`) requires the
*  block's page to be a journal, so a range wider than every journal date IS
*  "is on a journal page", and OG reads the text the same way Tine does
*  (GH #619, item 3). The legacy `(journal)` text Tine used to write still
*  reads as this condition ({@link builderLeafKind}). */
export function journalFilter(): Filter {
  return throughPage(boundedFilter("day", JOURNAL_ANY_RANGE[0], JOURNAL_ANY_RANGE[1]));
}

/** Whether `pred` (a page-row predicate) is the wide journal range. */
function isJournalAnyRange(pred: Filter): boolean {
  const leaf = asAttrLeaf(pred);
  if (leaf?.attr !== "day" || leaf.op !== "between") return false;
  if (leaf.value.kind !== "list" || leaf.value.items.length !== 2) return false;
  return (
    dateOf(leaf.value.items[0]) === JOURNAL_ANY_RANGE[0] &&
    dateOf(leaf.value.items[1]) === JOURNAL_ANY_RANGE[1]
  );
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

/** The `props` predicate's key and its atom test, mirroring `Filter::props_key` /
*  `props_atom_test` on the Rust side (one shared golden pins both): the key is
*  the ONE direct key-equality item; everything else is the atom test, an `and`
*  when there are several. Two keys is no property predicate. */
export function propsParts(pred: Filter): { key: string; atom: Filter | null } | null {
  const items = pred.kind === "and" ? pred.items : [pred];
  let key: string | null = null;
  const atoms: Filter[] = [];
  for (const item of items) {
    const leaf = asAttrLeaf(item);
    if (leaf && leaf.attr === "key" && leaf.op === "eq") {
      const text = textOf(leaf.value);
      if (text == null || key != null) return null;
      key = text;
      continue;
    }
    atoms.push(item);
  }
  if (key == null) return null;
  return { key, atom: atoms.length === 0 ? null : atoms.length === 1 ? atoms[0] : { kind: "and", items: atoms } };
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

/** Which date a `between`-kind row ranges over, so the row can say "Scheduled" or "Deadline" instead of
 *  the generic "Between dates" (GH #619 item 5). `null` for any other shape. */
export function betweenRowField(filter: Filter): BetweenField | null {
  if (builderLeafKind(filter) !== "between") return null;
  const page = asRelLeaf(filter, "page");
  const leaf = asAttrLeaf(page ? page.pred : filter);
  if (!leaf) return null;
  if (leaf.attr === "scheduled" || leaf.attr === "deadline") return leaf.attr;
  return leaf.attr === "day" ? "journal" : null;
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
    if (isJournalAnyRange(page.pred)) return "journal";
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
      if (["eq", "gt", "ge", "lt", "le", "is_not_set"].includes(leaf.op)) return leaf.attr;
      return leaf.op === "between" || leaf.op === "ge" || leaf.op === "le" ? "between" : null;
    case "content":
      if (leaf.op === "like") return "content";
      return leaf.op === "match" ? "search" : null;
    default:
      return null;
  }
}

// Human-readable chip labels. These are deferred vocabulary, never DSL tokens.
const ATTR_PHRASE: Record<Attr, Msg> = {
  content: msg("query-prose-attr-content"),
  task: msg("query-prose-attr-task"),
  priority: msg("query-prose-attr-priority"),
  scheduled: msg("query-prose-attr-scheduled"),
  deadline: msg("query-prose-attr-deadline"),
  created_at: msg("query-prose-attr-created-at"),
  last_modified_at: msg("query-prose-attr-last-modified-at"),
  name: msg("query-prose-attr-name"),
  journal: msg("query-prose-attr-journal"),
  day: msg("query-prose-attr-day"),
  namespace: msg("query-prose-attr-namespace"),
  used_as_tag: msg("query-prose-attr-used-as-tag"),
  key: msg("query-prose-attr-key"),
  value: msg("query-prose-attr-value"),
  atom_count: msg("query-prose-attr-atom-count"),
};

function valuePhrase(value: Value): string {
  switch (value.kind) {
    case "text": return value.text;
    case "number": return String(value.number);
    case "date": return value.literal;
    case "bool": return t("query-prose-boolean", { value: String(value.bool) });
    case "list": return value.items.map(valuePhrase).join(" | ");
    case "none": return "";
  }
}

/** Display relative bounds; authored dates and journal titles pass through. */
function boundWords(token: string): string {
  const tokenText = token.trim();
  switch (tokenText.toLowerCase()) {
    case "today":
    case "now": return t("query-prose-today");
    case "yesterday": return t("query-prose-yesterday");
    case "tomorrow": return t("query-prose-tomorrow");
  }
  const relative = /^([+-]?)(\d+)([dwmy])$/i.exec(tokenText);
  if (!relative) return tokenText;
  const count = parseInt(relative[2], 10);
  if (count === 0) return t("query-prose-today");
  const messages = {
    d: "query-prose-relative-days", w: "query-prose-relative-weeks",
    m: "query-prose-relative-months", y: "query-prose-relative-years",
  } as const;
  return t(messages[relative[3].toLowerCase() as keyof typeof messages], { count, direction: relative[1] === "-" ? "ago" : "ahead" });
}

function rangeWords(low: string, high: string): string {
  const preset = DATE_PRESETS.find(p => p.start === low.trim() && p.end === high.trim());
  if (preset) return preset.label.toLowerCase();
  return t("query-prose-date-range", { low: boundWords(low), high: boundWords(high) });
}

function datePhrase(attr: Attr, leaf: Leaf & { kind: "attr" }): PhraseSegment[] | null {
  const field = attr === "day" ? t("query-prose-journal-date-field") : render(ATTR_PHRASE[attr]);
  if (leaf.op === "between" && leaf.value.kind === "list" && leaf.value.items.length === 2) {
    const [low, high] = leaf.value.items.map(item => dateOf(item) ?? valuePhrase(item));
    return phrase("query-prose-field-value", { field: [words(field)], value: [chip(rangeWords(low, high))] });
  }
  if ((leaf.op === "ge" || leaf.op === "le") && leaf.value.kind === "date") {
    const value = t("query-prose-date-bound", { op: leaf.op, when: boundWords(leaf.value.literal) });
    return phrase("query-prose-field-value", { field: [words(field)], value: [chip(value)] });
  }
  return null;
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

/**
 * Fluent owns the complete clause and can reorder every argument. Segment
 * arguments travel as opaque placeholders; authored content is never parsed as
 * markup or interpolated back into the format string. Restore the original
 * segments after formatting so chips, fields and advanced cues keep their roles.
 */
type PhraseParts = (PhraseSegment | PhraseParts)[];

function formatPhrase<I extends MessageId>(id: I, args: { [K in keyof MessageArgs[I]]: string | number | PhraseParts }): PhraseParts {
  const slots: PhraseParts[] = [];
  const values: Record<string, string | number> = {};
  for (const [key, value] of Object.entries(args as Record<string, string | number | PhraseParts>)) {
    if (Array.isArray(value)) {
      values[key] = "\uFFF0" + slots.length + "\uFFF1";
      slots.push(value);
    } else values[key] = value;
  }
  // The public helper's mapped arguments retain the generated key checks;
  // only the internal segment-to-string representation crosses this cast.
  const text = (t as (id: MessageId, args: Record<string, string | number>) => string)(id, values);
  const out: PhraseParts = [];
  const pattern = /\uFFF0(\d+)\uFFF1/g;
  let start = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index! > start) out.push(words(text.slice(start, match.index)));
    out.push(slots[Number(match[1])]);
    start = match.index! + match[0].length;
  }
  if (start < text.length) out.push(words(text.slice(start)));
  return out;
}

/** Iterative flatten: wide lists stay linear and cannot exhaust the stack. */
function flattenPhrase(parts: PhraseParts): PhraseSegment[] {
  const out: PhraseSegment[] = [];
  const pending = [...parts].reverse();
  while (pending.length) {
    const part = pending.pop()!;
    if (Array.isArray(part)) for (let i = part.length - 1; i >= 0; i--) pending.push(part[i]);
    else out.push(part);
  }
  return out;
}

function phrase<I extends MessageId>(id: I, args: { [K in keyof MessageArgs[I]]: string | number | PhraseSegment[] }): PhraseSegment[] {
  return flattenPhrase(formatPhrase(id, args));
}

export const ADVANCED_PHRASE = msg("query-prose-advanced");
const advanced = (): PhraseSegment[] => [seg("advanced", render(ADVANCED_PHRASE))];

/** One bounded producer for the row, value cell and resting sentence. */
export function filterPhrase(filter: Filter, depth = 0): PhraseSegment[] {
  if (depth >= MAX_QUERY_BUILDER_DEPTH) return advanced();
  switch (filter.kind) {
    case "and":
    case "or": return [words(t("query-prose-group", { kind: filter.kind }))];
    case "not": return [words(t("query-prose-not-group"))];
    case "off": return phrase("query-prose-off", { inner: filterPhrase(filter.inner, depth + 1) });
    case "raw": return [chip(filter.text)];
    case "true": return [words(t("query-prose-everything"))];
    case "false": return [words(t("query-prose-nothing"))];
    case "leaf": return leafPhrase(filter, filter.leaf, depth);
  }
}

export function filterLabel(filter: Filter): string {
  return filterPhrase(filter).map(segment => segment.text).join("");
}

export function filterValueLabel(filter: Filter): string {
  const segments = filterPhrase(filter);
  const values = segments.filter(segment => segment.kind === "value");
  return values.length ? values.map(segment => segment.text).join(" ") : filterLabel(filter);
}

function leafPhrase(filter: Filter, leaf: Leaf, depth: number): PhraseSegment[] {
  const kind = builderLeafKind(filter);
  if (leaf.kind === "rel") {
    const inner = asAttrLeaf(leaf.pred);
    const innerText = inner ? (textOf(inner.value) ?? "") : "";
    switch (kind) {
      case "page": return [chip(innerText)];
      case "onPage": return phrase("query-prose-on-page", { value: [chip(innerText)] });
      case "namespace": return phrase("query-prose-in-namespace", { value: [chip(innerText.replace(/\/$/, ""))] });
      case "journal": return [words(t("query-prose-on-journal"))];
    }
    if (leaf.rel === "page") {
      const inner = filterPhrase(leaf.pred, kind ? depth : depth + 1);
      return kind === "pageProperty" ? phrase("query-prose-page-property-clause", { inner }) : inner;
    }
    if (leaf.rel === "props") {
      const parts = propsParts(leaf.pred);
      if (parts) {
        const atom = parts.atom ? asAttrLeaf(parts.atom) : null;
        if (parts.key === "tags" && atom?.op === "in") {
          return phrase("query-prose-page-tags", { value: [chip((listOf(atom.value) ?? []).join(" | "))] });
        }
        if (!parts.atom) return phrase("query-prose-property-any", { field: [named(parts.key)] });
        if (atom?.op === "eq") return phrase("query-prose-field-value", { field: [named(parts.key)], value: [chip(valuePhrase(atom.value))] });
      }
      const test = propertyLeafTest(filter);
      if (test) return propertyTestPhrase(test);
    }
    return phrase("query-prose-relation", { quant: leaf.quant, relation: leaf.rel, inner: filterPhrase(leaf.pred, depth + 1) });
  }
  switch (kind) {
    case "task": {
      const markers = listOf(leaf.value) ?? [];
      const value = isAnyTaskStatus(markers) ? t("query-prose-any-status") : markers.join(" | ") || t("query-prose-no-markers");
      return phrase("query-prose-task", { value: [chip(value)] });
    }
    case "priority": return phrase("query-prose-priority", { value: [chip((listOf(leaf.value) ?? []).join(" | ") || t("query-prose-no-levels"))] });
    case "scheduled":
    case "deadline":
      if (leaf.op === "is_set") return [words(render(ATTR_PHRASE[leaf.attr]))];
      break;
    case "content": return phrase("query-prose-content", { value: [chip(plainLikeSubstring(textOf(leaf.value) ?? "") ?? valuePhrase(leaf.value))] });
    case "search": return phrase("query-prose-search", { value: [chip(valuePhrase(leaf.value))] });
  }
  if (leaf.attr === "day" || leaf.attr === "scheduled" || leaf.attr === "deadline") {
    const result = datePhrase(leaf.attr, leaf);
    if (result) return result;
  }
  return phrase("query-prose-attribute-test", { field: [named(render(ATTR_PHRASE[leaf.attr]))], op: leaf.op, value: [chip(valuePhrase(leaf.value))] });
}

function propertyTestPhrase(test: PropertyLeafTest): PhraseSegment[] {
  return phrase("query-prose-property-test", { field: [named(test.key)], op: test.id, value: [chip(test.values.join(" ~ "))] });
}

export function isEmptyFilter(filter: Filter): boolean {
  return filter.kind === "true" || (filter.kind === "and" && filter.items.length === 0);
}

function joinPhrases(parts: PhraseSegment[][], connector: "and" | "or"): PhraseSegment[] {
  if (parts.length === 0) return [];
  if (parts.length === 1) return parts[0];
  // List punctuation belongs to the locale too; fold complete clauses rather
  // than appending translated connector fragments in an English word order.
  let out: PhraseParts = parts[0];
  for (let i = 1; i < parts.length; i++) {
    out = formatPhrase("query-prose-list", {
      left: out, right: parts[i], connector,
      position: i === parts.length - 1 ? (parts.length > 2 ? "last-many" : "last") : "middle",
    });
  }
  return flattenPhrase(out);
}

function clausePhrase(filter: Filter, depth: number, isRoot: boolean): PhraseSegment[] {
  if (depth >= MAX_QUERY_BUILDER_DEPTH) return advanced();
  switch (filter.kind) {
    case "and":
    case "or": {
      if (filter.items.length === 0) return filterPhrase(filter, depth);
      if (filter.items.length === 1) return clausePhrase(filter.items[0], depth + 1, isRoot);
      const inner = joinPhrases(filter.items.map(item => clausePhrase(item, depth + 1, false)), filter.kind);
      return isRoot ? inner : phrase("query-prose-parentheses", { inner });
    }
    case "not":
    case "off": {
      const inner = clausePhrase(filter.inner, isLeafLike(filter.inner) ? depth : depth + 1, false);
      return phrase(filter.kind === "not" ? "query-prose-negation" : "query-prose-off", { inner });
    }
    default: return filterPhrase(filter, depth);
  }
}

/** Resting text for a whole query; the locale owns subject/predicate order. */
export function querySentence(query: { anchor: Anchor; filter: Filter; both?: boolean }): PhraseSegment[] {
  const family = query.both ? "both" : query.anchor;
  if (isEmptyFilter(query.filter)) return [words(t("query-prose-all", { family }))];
  return phrase("query-prose-sentence", {
    subject: [named(t("query-prose-subject", { family }))],
    predicate: clausePhrase(query.filter, 0, true),
  });
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
// Immutable tree edits live in ./queryTree (re-exported here: one import site).
export {
  filterChildren,
  builderRoot,
  addChild,
  removeAt,
  replaceAt,
  wrapAt,
  groupSelected,
  groupWithPrevious,
  moveSibling,
  moveAcross,
  isDisabledAt,
  toggleDisabledAt,
  unwrapAt,
  setOp,
} from "./queryTree";
export type { GroupChoice } from "./queryTree";
