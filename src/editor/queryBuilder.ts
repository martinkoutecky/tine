// Query builder model: parse a `{{query ...}}` DSL string into an editable
// filter tree, mutate it, and serialize back to DSL. Pure + unit-testable (no
// DOM). Mirrors OG Logseq's handler/query/builder.cljs (from-dsl / ->dsl /
// add/remove/wrap/unwrap) but scoped to the DSL subset Tine's engine actually
// runs (see crates/tine-core/src/query.rs): page/tag refs, and/or/not, task,
// priority, property, scheduled, deadline, between.
//
// Single source of truth is the block's DSL text. The UI parses it to a tree,
// applies an immutable mutation, serializes back, and writes the block — so the
// tree never drifts from what's stored.

import { MARKERS as TASK_MARKERS } from "../markers";

export type Clause =
  | { kind: "op"; op: "and" | "or" | "not"; children: Clause[] }
  | { kind: "page"; name: string } // a [[page]] / #tag / (page-ref) reference
  | { kind: "task"; markers: string[] }
  | { kind: "priority"; levels: string[] }
  | { kind: "property"; key: string; value: string | null }
  | { kind: "scheduled" }
  | { kind: "deadline" }
  | { kind: "journal" } // block lives on a journal page
  | { kind: "between"; field: BetweenField; start: string; end: string }
  | { kind: "onPage"; name: string } // (page name) — blocks on a named page
  | { kind: "namespace"; ns: string }
  | { kind: "pageProperty"; key: string; value: string | null }
  | { kind: "pageTags"; tags: string[] }
  | { kind: "content"; text: string }
  | { kind: "sortBy"; field: string; dir: "asc" | "desc" } // result ordering (query-global)
  // Result-level aggregation/grouping, computed in the frontend from the returned
  // block list (see Macro.tsx). Ride in the DSL so the builder round-trips and the
  // Rust engine returns the full set (it parses these as no-op filters).
  | { kind: "aggregate"; agg: "count" | "sum" | "avg"; field: string | null }
  | { kind: "groupBy"; field: string }
  // Verbatim fallback for a sub-expression we don't model, so an unfamiliar
  // (but non-datalog) query round-trips losslessly instead of being discarded.
  | { kind: "raw"; text: string };

// Which date a `between` range tests against. "any" = the permissive default
// (journal date OR scheduled OR deadline); "journal" matches OG's journal-only
// `between`. Serialized as a leading keyword for everything but "any".
export type BetweenField = "any" | "journal" | "scheduled" | "deadline";
export const BETWEEN_FIELDS: BetweenField[] = ["journal", "scheduled", "deadline", "any"];

// The full task-marker set (src/markers.ts) as a mutable array for the picker.
// Was a hand-copied 7 that omitted WAIT / CANCELLED / IN-PROGRESS, so those tasks
// couldn't be filtered though blocks could be marked with them.
export const MARKERS: string[] = [...TASK_MARKERS];
export const PRIORITIES = ["A", "B", "C"];

/** A date bound that resolves on its own (keyword / relative / ISO) is written
 *  bare; a journal page title is wrapped in `[[ ]]` (matching OG). */
function isBareDateToken(s: string): boolean {
  return (
    /^(today|yesterday|tomorrow|now)$/i.test(s) ||
    /^[+-]?\d+[dwmy]$/i.test(s) ||
    /^\d{4}-\d{2}-\d{2}$/.test(s)
  );
}
function dateBound(s: string): string {
  return isBareDateToken(s.trim()) ? s.trim() : `[[${s.trim()}]]`;
}

// ---------------------------------------------------------------------------
// Tokenizer (mirrors query.rs::tokenize, plus source spans for raw capture)
// ---------------------------------------------------------------------------

type Tok =
  | { t: "("; s: number; e: number }
  | { t: ")"; s: number; e: number }
  | { t: "page"; v: string; s: number; e: number }
  | { t: "tag"; v: string; s: number; e: number }
  | { t: "word"; v: string; s: number; e: number }
  | { t: "str"; v: string; s: number; e: number };

function tokenize(src: string): Tok[] {
  const toks: Tok[] = [];
  const ch = Array.from(src);
  let i = 0;
  while (i < ch.length) {
    const c = ch[i];
    if (/\s/.test(c)) {
      i++;
    } else if (c === "(") {
      toks.push({ t: "(", s: i, e: i + 1 });
      i++;
    } else if (c === ")") {
      toks.push({ t: ")", s: i, e: i + 1 });
      i++;
    } else if (c === "[" && ch[i + 1] === "[") {
      let j = i + 2;
      let name = "";
      while (j + 1 < ch.length && !(ch[j] === "]" && ch[j + 1] === "]")) name += ch[j++];
      toks.push({ t: "page", v: name, s: i, e: j + 2 });
      i = j + 2;
    } else if (c === "#") {
      if (ch[i + 1] === "[" && ch[i + 2] === "[") {
        let j = i + 3;
        let name = "";
        while (j + 1 < ch.length && !(ch[j] === "]" && ch[j + 1] === "]")) name += ch[j++];
        toks.push({ t: "tag", v: name, s: i, e: j + 2 });
        i = j + 2;
      } else {
        let j = i + 1;
        let name = "";
        while (j < ch.length && /[\w/.-]/.test(ch[j])) name += ch[j++];
        toks.push({ t: "tag", v: name, s: i, e: j });
        i = j;
      }
    } else if (c === '"') {
      let j = i + 1;
      let s = "";
      // Escape-aware: ONLY `\"` and `\\` are escapes (→ literal quote/backslash),
      // so a quote inside the value doesn't end the string early. A backslash
      // before any other char is kept literally, so a hand-authored path like
      // `"C:\tmp"` round-trips unchanged (mirrors query.rs::tokenize).
      while (j < ch.length && ch[j] !== '"') {
        if (ch[j] === "\\" && (ch[j + 1] === '"' || ch[j + 1] === "\\")) {
          s += ch[j + 1];
          j += 2;
        } else {
          s += ch[j++];
        }
      }
      toks.push({ t: "str", v: s, s: i, e: j + 1 });
      i = j + 1;
    } else {
      let j = i;
      let w = "";
      while (j < ch.length && !/\s/.test(ch[j]) && ch[j] !== "(" && ch[j] !== ")") w += ch[j++];
      toks.push({ t: "word", v: w, s: i, e: j });
      i = j;
    }
  }
  return toks;
}

// ---------------------------------------------------------------------------
// Parser (DSL string -> Clause). Returns null on a form we don't recognise.
// ---------------------------------------------------------------------------

interface Cur {
  pos: number;
}

function parseExpr(toks: Tok[], cur: Cur, src: string): Clause | null {
  const t = toks[cur.pos];
  if (!t) return null;
  if (t.t === "page" || t.t === "tag") {
    cur.pos++;
    return { kind: "page", name: t.v };
  }
  // A bare quoted string is a full-text content filter.
  if (t.t === "str") {
    cur.pos++;
    return { kind: "content", text: t.v };
  }
  if (t.t === "(") {
    const open = t;
    cur.pos++;
    const head = toks[cur.pos];
    if (!head || head.t !== "word") return null;
    const name = head.v.toLowerCase();
    cur.pos++;
    let clause: Clause | null = null;
    switch (name) {
      case "and":
      case "or":
        clause = { kind: "op", op: name, children: parseList(toks, cur, src) };
        break;
      case "not": {
        const child = parseExpr(toks, cur, src);
        clause = child ? { kind: "op", op: "not", children: [child] } : null;
        break;
      }
      case "task":
      case "todo":
        clause = { kind: "task", markers: parseWords(toks, cur) };
        break;
      case "priority":
        clause = { kind: "priority", levels: parseWords(toks, cur) };
        break;
      case "page-ref": {
        const n = parseName(toks, cur);
        clause = n != null ? { kind: "page", name: n } : null;
        break;
      }
      case "page": {
        const n = parseName(toks, cur);
        clause = n != null ? { kind: "onPage", name: n } : null;
        break;
      }
      case "namespace": {
        const n = parseName(toks, cur);
        clause = n != null ? { kind: "namespace", ns: n } : null;
        break;
      }
      case "property": {
        const key = parseName(toks, cur);
        if (key == null) {
          clause = null;
          break;
        }
        const value = parseOptValue(toks, cur);
        clause = { kind: "property", key: normPropKey(key), value };
        break;
      }
      case "page-property": {
        const key = parseName(toks, cur);
        if (key == null) {
          clause = null;
          break;
        }
        const value = parseOptValue(toks, cur);
        clause = { kind: "pageProperty", key: normPropKey(key), value };
        break;
      }
      case "page-tags":
      case "tags":
        clause = { kind: "pageTags", tags: parseWords(toks, cur) };
        break;
      case "scheduled":
        clause = { kind: "scheduled" };
        break;
      case "deadline":
        clause = { kind: "deadline" };
        break;
      case "journal":
        clause = { kind: "journal" };
        break;
      case "between": {
        // Optional leading field keyword (journal/scheduled/deadline).
        let field: BetweenField = "any";
        const peek = toks[cur.pos];
        if (peek && peek.t === "word" && ["journal", "scheduled", "deadline"].includes(peek.v.toLowerCase())) {
          field = peek.v.toLowerCase() as BetweenField;
          cur.pos++;
        }
        const start = parseName(toks, cur) ?? "";
        const end = parseName(toks, cur) ?? "";
        clause = { kind: "between", field, start, end };
        break;
      }
      case "sort-by": {
        const field = parseName(toks, cur);
        if (field == null) {
          clause = null;
          break;
        }
        const dir = parseOptName(toks, cur);
        clause = { kind: "sortBy", field, dir: dir?.toLowerCase() === "desc" ? "desc" : "asc" };
        break;
      }
      case "aggregate": {
        const k = parseName(toks, cur)?.toLowerCase();
        if (k === "sum") clause = { kind: "aggregate", agg: "sum", field: parseName(toks, cur) };
        else if (k === "avg" || k === "average")
          clause = { kind: "aggregate", agg: "avg", field: parseName(toks, cur) };
        else clause = { kind: "aggregate", agg: "count", field: null };
        break;
      }
      case "group-by":
        clause = { kind: "groupBy", field: parseName(toks, cur) ?? "page" };
        break;
      default:
        clause = null;
    }
    // Consume up to and including THIS form's matching ")", tracking nested
    // parens — the opening "(" consumed above puts us at depth 1. (Parens inside
    // strings/page-refs are folded into str/page tokens, so they never count.)
    // A lazy stop at the first ")" would split an unknown NESTED form like
    // `(custom (nested x))` at the inner ")", orphaning later siblings and
    // emitting an unbalanced raw fragment that corrupts the query on re-serialize.
    let depth = 1;
    let close: Tok | undefined;
    while (cur.pos < toks.length) {
      const tk = toks[cur.pos];
      if (tk.t === "(") depth++;
      else if (tk.t === ")") {
        depth--;
        if (depth === 0) {
          close = tk;
          cur.pos++;
          break;
        }
      }
      cur.pos++;
    }
    if (clause == null && close) {
      // Unknown form: preserve it verbatim (balanced) so it round-trips.
      return { kind: "raw", text: src.slice(open.s, close.e) };
    }
    return clause;
  }
  // A bare word/string at expression position isn't part of Tine's runnable
  // grammar; skip it.
  cur.pos++;
  return null;
}

function parseList(toks: Tok[], cur: Cur, src: string): Clause[] {
  const out: Clause[] = [];
  while (toks[cur.pos] && toks[cur.pos].t !== ")") {
    const before = cur.pos;
    const c = parseExpr(toks, cur, src);
    if (c) out.push(c);
    if (cur.pos === before) cur.pos++; // guard against non-advance
  }
  return out;
}

function parseWords(toks: Tok[], cur: Cur): string[] {
  const out: string[] = [];
  for (;;) {
    const t = toks[cur.pos];
    if (t && (t.t === "word" || t.t === "str" || t.t === "tag" || t.t === "page")) {
      out.push(t.v);
      cur.pos++;
    } else break;
  }
  return out;
}

function parseName(toks: Tok[], cur: Cur): string | null {
  const t = toks[cur.pos];
  if (t && (t.t === "word" || t.t === "str" || t.t === "page" || t.t === "tag")) {
    cur.pos++;
    return t.v;
  }
  return null;
}

function parseOptName(toks: Tok[], cur: Cur): string | null {
  const t = toks[cur.pos];
  if (t && (t.t === "word" || t.t === "str")) return parseName(toks, cur);
  return null;
}

/** A property KEY normalized as Logseq's query DSL does (mirrors
 *  query.rs::normalize_prop_key): drop a leading `:` (keyword form `:fach` ==
 *  symbol form `fach`) and map `_`→`-`. */
function normPropKey(k: string): string {
  return k.replace(/^:+/, "").replace(/_/g, "-");
}

/** Optional property VALUE: like `parseOptName` but also accepts a `[[page]]` /
 *  `#tag` token (mirrors query.rs::parse_opt_value) so `(property k [[Page]])`
 *  keeps its value instead of dropping it and leaking a stray page-ref clause. */
function parseOptValue(toks: Tok[], cur: Cur): string | null {
  const t = toks[cur.pos];
  if (t && (t.t === "word" || t.t === "str" || t.t === "page" || t.t === "tag"))
    return parseName(toks, cur);
  return null;
}

/** Parse a query DSL body into a root op node (always `and`/`or`). An empty or
 *  unparseable-at-top body yields an empty `and` root. */
export function parseQuery(dsl: string): Clause {
  const src = dsl.trim();
  if (src === "") return { kind: "op", op: "and", children: [] };
  const toks = tokenize(src);
  const cur: Cur = { pos: 0 };
  const expr = parseExpr(toks, cur, src);
  if (!expr) return { kind: "op", op: "and", children: [{ kind: "raw", text: src }] };
  if (expr.kind === "op" && (expr.op === "and" || expr.op === "or")) return expr;
  return { kind: "op", op: "and", children: [expr] };
}

// ---------------------------------------------------------------------------
// Serializer (Clause -> DSL string)
// ---------------------------------------------------------------------------

// Wrap a value in a DSL double-quoted string, escaping `\` and `"` so a value
// containing a quote (or backslash) round-trips faithfully — the tokenizer
// below (and query.rs::tokenize) unescape the same way. Without this, a value
// like `foo "bar"` serialized to `"foo "bar""` and silently re-parsed as `foo `.
function quoteStr(s: string): string {
  return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}
// Quote when the value can't be a bare word: it has whitespace, is empty, or
// contains a DSL metacharacter (`(`/`)` would break the paren structure, `"`
// would start/stop a string mid-word).
function needsQuote(s: string): boolean {
  return s === "" || /[\s()"]/.test(s);
}
function word(s: string): string {
  return needsQuote(s) ? quoteStr(s) : s;
}

function clauseDsl(c: Clause): string {
  switch (c.kind) {
    case "page":
      return `[[${c.name}]]`;
    case "task":
      return c.markers.length ? `(task ${c.markers.join(" ")})` : "(task)";
    case "priority":
      return c.levels.length ? `(priority ${c.levels.join(" ")})` : "(priority)";
    case "property":
      return c.value != null && c.value !== ""
        ? `(property ${word(c.key)} ${word(c.value)})`
        : `(property ${word(c.key)})`;
    case "scheduled":
      return "(scheduled)";
    case "deadline":
      return "(deadline)";
    case "journal":
      return "(journal)";
    case "between": {
      const f = c.field && c.field !== "any" ? `${c.field} ` : "";
      return `(between ${f}${dateBound(c.start)} ${dateBound(c.end)})`;
    }
    case "onPage":
      return `(page ${word(c.name)})`;
    case "namespace":
      return `(namespace ${word(c.ns)})`;
    case "pageProperty":
      return c.value != null && c.value !== ""
        ? `(page-property ${word(c.key)} ${word(c.value)})`
        : `(page-property ${word(c.key)})`;
    case "pageTags":
      return `(page-tags ${c.tags.join(" ")})`;
    case "content":
      return quoteStr(c.text);
    case "sortBy":
      return `(sort-by ${word(c.field)} ${c.dir})`;
    case "aggregate":
      return c.agg === "count"
        ? "(aggregate count)"
        : `(aggregate ${c.agg} ${word(c.field ?? "")})`;
    case "groupBy":
      return `(group-by ${word(c.field)})`;
    case "raw":
      return c.text;
    case "op": {
      const kids = c.children.map(clauseDsl);
      if (c.op === "not") return `(not ${kids[0] ?? ""})`;
      return `(${c.op} ${kids.join(" ")})`;
    }
  }
}

/** Serialize the root to a DSL body. Simplifies a single-child `and` to just
 *  the child (matching OG's simplify-query); an empty root yields "". */
export function toDsl(root: Clause): string {
  if (root.kind !== "op") return clauseDsl(root);
  const kids = root.children;
  if (root.op === "and") {
    if (kids.length === 0) return "";
    if (kids.length === 1) return clauseDsl(kids[0]);
  }
  return clauseDsl(root);
}

// Pre-conversion simple DSL, kept so the "← Simple" toggle can restore the exact
// query (incl. sort/aggregate/group-by that clauseToAdvanced drops) within a session.
const simpleFormStash = new Map<string, string>();
export function stashSimpleForm(blockId: string, dsl: string): void {
  simpleFormStash.set(blockId, dsl);
}
export function getSimpleForm(blockId: string): string | undefined {
  return simpleFormStash.get(blockId);
}
export function clearSimpleForm(blockId: string): void {
  simpleFormStash.delete(blockId);
}

// ---------------------------------------------------------------------------
// Simple-DSL → advanced (Datalog) conversion, for the builder's "⚙ advanced"
// pill. Two hard rules learned from the Jul 8 data-mutation bug:
//  1. The output MUST be SINGLE-LINE and BRACE-FREE — lsdoc macros
//     ({{query …}}) never span lines AND their args cannot contain `{`/`}`
//     (a `#{…}` EDN set turns the whole macro into plain text + a stray tag),
//     so no `;;` comments and no set literals. `run_advanced_query` collects
//     plain quoted strings identically (adv_strings ignores the set wrapper).
//  2. NEVER discard the user's query: filters convert 1:1 to the clause heads
//     `run_advanced_query` accepts; result-shaping (sort/aggregate/group-by)
//     has no datalog home and is reported as dropped; a clause with no
//     advanced equivalent makes the whole conversion REFUSE rather than lose
//     content.
// ---------------------------------------------------------------------------

export type AdvancedConversion =
  | { ok: true; dsl: string; dropped: string[] }
  | { ok: false; unsupported: string[] };

export function clauseToAdvanced(root: Clause): AdvancedConversion {
  const unsupported: string[] = [];
  const dropped: string[] = [];
  const strs = (xs: string[]) => xs.map(quoteStr).join(" ");
  const emit = (c: Clause): string | null => {
    switch (c.kind) {
      case "op": {
        const kids = c.children.map(emit).filter((s): s is string => !!s);
        if (!kids.length) return null;
        if (c.op === "not") return `(not ${kids[0]})`;
        if (kids.length === 1) return kids[0];
        return `(${c.op} ${kids.join(" ")})`;
      }
      case "page":
        return `(page-ref ?b ${quoteStr(c.name)})`;
      case "task":
        return c.markers.length ? `(task ?b ${strs(c.markers)})` : `(task ?b)`;
      case "priority":
        return c.levels.length ? `(priority ?b ${strs(c.levels)})` : `(priority ?b)`;
      case "property":
        return c.value === null
          ? `(property ?b :${c.key})`
          : `(property ?b :${c.key} ${quoteStr(c.value)})`;
      case "pageProperty":
        return c.value === null
          ? `(page-property ?b :${c.key})`
          : `(page-property ?b :${c.key} ${quoteStr(c.value)})`;
      case "pageTags":
        return c.tags.length ? `(page-tags ?b ${strs(c.tags)})` : null;
      case "scheduled":
        return `(scheduled ?b)`;
      case "deadline":
        return `(deadline ?b)`;
      case "journal":
        return `(journal ?b)`;
      case "onPage":
        return `(page ?b ${quoteStr(c.name)})`;
      case "namespace":
        return `(namespace ?b ${quoteStr(c.ns)})`;
      case "between": {
        const bounds = `?b ${quoteStr(c.start)} ${quoteStr(c.end)}`;
        // "any" (simple default: journal OR scheduled OR deadline) has no
        // single advanced head — expand it to the faithful (or …).
        if (c.field === "any") {
          return `(or (between :journal ${bounds}) (between :scheduled ${bounds}) (between :deadline ${bounds}))`;
        }
        return c.field === "journal" ? `(between ${bounds})` : `(between :${c.field} ${bounds})`;
      }
      case "sortBy":
        dropped.push(`sort by ${c.field}`);
        return null;
      case "aggregate":
        dropped.push(c.field ? `${c.agg}(${c.field})` : c.agg);
        return null;
      case "groupBy":
        dropped.push(`group by ${c.field}`);
        return null;
      case "content":
        unsupported.push(`full-text ${quoteStr(c.text)}`);
        return null;
      case "raw":
        unsupported.push(c.text);
        return null;
    }
  };
  // Flatten a top-level (and …) into sibling :where groups — datalog groups
  // are implicitly and-ed, and it reads like the cheat-sheet examples.
  const groups =
    root.kind === "op" && root.op === "and"
      ? root.children.map(emit).filter((s): s is string => !!s)
      : [emit(root)].filter((s): s is string => !!s);
  if (unsupported.length) return { ok: false, unsupported };
  const body = groups.length ? groups.join(" ") : `(task ?b "TODO" "DOING")`;
  return { ok: true, dsl: `[:find (pull ?b [*]) :where ${body}]`, dropped };
}

// ---------------------------------------------------------------------------
// Advanced (Datalog) → simple-DSL conversion. This is intentionally strict: it
// only accepts the single-line `clauseToAdvanced` shape and the closed set of
// supported clause heads. Arbitrary Datalog must stay raw text.
// ---------------------------------------------------------------------------

type AdvancedTok =
  | { t: "(" | ")" | "[" | "]" }
  | { t: "word"; v: string }
  | { t: "str"; v: string };

function tokenizeAdvanced(src: string): AdvancedTok[] | null {
  const toks: AdvancedTok[] = [];
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (/\s/.test(c)) {
      i++;
    } else if (c === "(" || c === ")" || c === "[" || c === "]") {
      toks.push({ t: c });
      i++;
    } else if (c === '"') {
      let j = i + 1;
      let s = "";
      while (j < src.length && src[j] !== '"') {
        if (src[j] === "\\" && (src[j + 1] === '"' || src[j + 1] === "\\")) {
          s += src[j + 1];
          j += 2;
        } else {
          s += src[j++];
        }
      }
      if (j >= src.length) return null;
      toks.push({ t: "str", v: s });
      i = j + 1;
    } else {
      let j = i;
      while (j < src.length && !/\s/.test(src[j]) && !"()[]".includes(src[j])) j++;
      if (j === i) return null;
      toks.push({ t: "word", v: src.slice(i, j) });
      i = j;
    }
  }
  return toks;
}

interface AdvancedCur {
  pos: number;
}

function takeAdvancedSym(toks: AdvancedTok[], cur: AdvancedCur, sym: "(" | ")" | "[" | "]"): boolean {
  if (toks[cur.pos]?.t !== sym) return false;
  cur.pos++;
  return true;
}

function takeAdvancedWord(toks: AdvancedTok[], cur: AdvancedCur): string | null {
  const t = toks[cur.pos];
  if (t?.t !== "word") return null;
  cur.pos++;
  return t.v;
}

function takeAdvancedStr(toks: AdvancedTok[], cur: AdvancedCur): string | null {
  const t = toks[cur.pos];
  if (t?.t !== "str") return null;
  cur.pos++;
  return t.v;
}

function expectAdvancedWord(toks: AdvancedTok[], cur: AdvancedCur, word: string): boolean {
  const t = toks[cur.pos];
  if (t?.t !== "word" || t.v !== word) return false;
  cur.pos++;
  return true;
}

function takeAdvancedValue(toks: AdvancedTok[], cur: AdvancedCur): string | null {
  const t = toks[cur.pos];
  if (t?.t === "str") {
    cur.pos++;
    return t.v;
  }
  if (t?.t === "word" && !t.v.startsWith("?") && !t.v.startsWith(":")) {
    cur.pos++;
    return t.v;
  }
  return null;
}

function collapseAnyBetween(children: Clause[]): Clause | null {
  if (children.length !== 3) return null;
  const fields: BetweenField[] = ["journal", "scheduled", "deadline"];
  const first = children[0];
  if (!first || first.kind !== "between") return null;
  for (let i = 0; i < fields.length; i++) {
    const c = children[i];
    if (
      !c ||
      c.kind !== "between" ||
      c.field !== fields[i] ||
      c.start !== first.start ||
      c.end !== first.end
    ) {
      return null;
    }
  }
  return { kind: "between", field: "any", start: first.start, end: first.end };
}

function parseAdvancedChildren(toks: AdvancedTok[], cur: AdvancedCur): Clause[] | null {
  const children: Clause[] = [];
  while (toks[cur.pos] && toks[cur.pos].t !== ")") {
    const child = parseAdvancedExpr(toks, cur);
    if (!child) return null;
    children.push(child);
  }
  return children.length ? children : null;
}

function parseAdvancedExpr(toks: AdvancedTok[], cur: AdvancedCur): Clause | null {
  if (!takeAdvancedSym(toks, cur, "(")) return null;
  const head = takeAdvancedWord(toks, cur);
  if (!head) return null;

  switch (head) {
    case "and":
    case "or": {
      const children = parseAdvancedChildren(toks, cur);
      if (!children || !takeAdvancedSym(toks, cur, ")")) return null;
      if (head === "or") return collapseAnyBetween(children) ?? { kind: "op", op: "or", children };
      return { kind: "op", op: "and", children };
    }
    case "not": {
      const child = parseAdvancedExpr(toks, cur);
      if (!child || !takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "op", op: "not", children: [child] };
    }
    case "page-ref": {
      if (!expectAdvancedWord(toks, cur, "?b")) return null;
      const name = takeAdvancedStr(toks, cur);
      if (name == null || !takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "page", name };
    }
    case "task": {
      if (!expectAdvancedWord(toks, cur, "?b")) return null;
      const markers: string[] = [];
      while (toks[cur.pos] && toks[cur.pos].t !== ")") {
        const marker = takeAdvancedValue(toks, cur);
        if (marker == null) return null;
        markers.push(marker);
      }
      if (!takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "task", markers };
    }
    case "priority": {
      if (!expectAdvancedWord(toks, cur, "?b")) return null;
      const levels: string[] = [];
      while (toks[cur.pos] && toks[cur.pos].t !== ")") {
        const level = takeAdvancedValue(toks, cur);
        if (level == null) return null;
        levels.push(level);
      }
      if (!takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "priority", levels };
    }
    case "property":
    case "page-property": {
      if (!expectAdvancedWord(toks, cur, "?b")) return null;
      const key = takeAdvancedWord(toks, cur);
      if (!key?.startsWith(":")) return null;
      let value: string | null = null;
      if (toks[cur.pos]?.t !== ")") {
        value = takeAdvancedStr(toks, cur);
        if (value == null) return null;
      }
      if (!takeAdvancedSym(toks, cur, ")")) return null;
      return head === "property"
        ? { kind: "property", key: normPropKey(key), value }
        : { kind: "pageProperty", key: normPropKey(key), value };
    }
    case "page-tags": {
      if (!expectAdvancedWord(toks, cur, "?b")) return null;
      const tags: string[] = [];
      while (toks[cur.pos] && toks[cur.pos].t !== ")") {
        const tag = takeAdvancedValue(toks, cur);
        if (tag == null) return null;
        tags.push(tag);
      }
      if (!tags.length || !takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "pageTags", tags };
    }
    case "scheduled":
      if (!expectAdvancedWord(toks, cur, "?b") || !takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "scheduled" };
    case "deadline":
      if (!expectAdvancedWord(toks, cur, "?b") || !takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "deadline" };
    case "journal":
      if (!expectAdvancedWord(toks, cur, "?b") || !takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "journal" };
    case "page": {
      if (!expectAdvancedWord(toks, cur, "?b")) return null;
      const name = takeAdvancedStr(toks, cur);
      if (name == null || !takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "onPage", name };
    }
    case "namespace": {
      if (!expectAdvancedWord(toks, cur, "?b")) return null;
      const ns = takeAdvancedStr(toks, cur);
      if (ns == null || !takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "namespace", ns };
    }
    case "between": {
      const first = takeAdvancedWord(toks, cur);
      let field: BetweenField;
      if (first === "?b") {
        field = "journal";
      } else if (
        first === ":journal" ||
        first === ":scheduled" ||
        first === ":deadline"
      ) {
        field = first.slice(1) as BetweenField;
        if (!expectAdvancedWord(toks, cur, "?b")) return null;
      } else {
        return null;
      }
      const start = takeAdvancedStr(toks, cur);
      const end = takeAdvancedStr(toks, cur);
      if (start == null || end == null || !takeAdvancedSym(toks, cur, ")")) return null;
      return { kind: "between", field, start, end };
    }
    default:
      return null;
  }
}

function parseAdvancedFind(toks: AdvancedTok[], cur: AdvancedCur): boolean {
  return (
    takeAdvancedSym(toks, cur, "[") &&
    expectAdvancedWord(toks, cur, ":find") &&
    takeAdvancedSym(toks, cur, "(") &&
    expectAdvancedWord(toks, cur, "pull") &&
    expectAdvancedWord(toks, cur, "?b") &&
    takeAdvancedSym(toks, cur, "[") &&
    expectAdvancedWord(toks, cur, "*") &&
    takeAdvancedSym(toks, cur, "]") &&
    takeAdvancedSym(toks, cur, ")") &&
    expectAdvancedWord(toks, cur, ":where")
  );
}

export function advancedToClause(datalog: string): Clause | null {
  const toks = tokenizeAdvanced(datalog.trim());
  if (!toks) return null;
  const cur: AdvancedCur = { pos: 0 };
  if (!parseAdvancedFind(toks, cur)) return null;
  const clauses: Clause[] = [];
  while (toks[cur.pos] && toks[cur.pos].t !== "]") {
    const c = parseAdvancedExpr(toks, cur);
    if (!c) return null;
    clauses.push(c);
  }
  if (!clauses.length || !takeAdvancedSym(toks, cur, "]") || cur.pos !== toks.length) return null;
  return clauses.length === 1 ? clauses[0] : { kind: "op", op: "and", children: clauses };
}

// ---------------------------------------------------------------------------
// Sort presets — the one-click sort options in the builder (single source of
// truth for the buttons AND the chip label). `field`/`dir` are the DSL values;
// the engine resolves them to a block facet: `priority` → `[#A]` marker, `page`
// → page name, `deadline`/`scheduled` → the planning date, and `modified` → a
// recency axis (journal pages by the day they represent, other pages by file
// mtime), so journal and non-journal todos interleave on one timeline.
// ---------------------------------------------------------------------------

export interface SortPreset {
  field: string;
  dir: "asc" | "desc";
  label: string;
  hint: string;
}
export const SORT_PRESETS: SortPreset[] = [
  { field: "modified", dir: "desc", label: "Newest first", hint: "Most recent first — journal pages by their date, others by when the file was last modified" },
  { field: "modified", dir: "asc", label: "Oldest first", hint: "Oldest first — journal pages by their date, others by file modified time" },
  { field: "priority", dir: "asc", label: "Priority A→C", hint: "Highest priority ([#A]) first; unprioritized last" },
  { field: "page", dir: "asc", label: "Page A→Z", hint: "Alphabetically by the page each result lives on" },
  { field: "deadline", dir: "asc", label: "Deadline", hint: "Soonest DEADLINE first; blocks without a deadline last" },
  { field: "scheduled", dir: "asc", label: "Scheduled", hint: "Soonest SCHEDULED first; blocks without one last" },
];

/** Friendly text for a sort clause — a matching preset's label, else `field ↑/↓`. */
export function sortLabel(field: string, dir: "asc" | "desc"): string {
  const p = SORT_PRESETS.find((p) => p.field === field && p.dir === dir);
  if (p) return p.label.toLowerCase();
  return `${field} ${dir === "desc" ? "↓" : "↑"}`;
}

// ---------------------------------------------------------------------------
// Human-readable chip labels
// ---------------------------------------------------------------------------

export function clauseLabel(c: Clause): string {
  switch (c.kind) {
    case "page":
      return c.name;
    case "task":
      return `task: ${c.markers.length ? c.markers.join(" | ") : "any"}`;
    case "priority":
      return `priority: ${c.levels.length ? c.levels.join(" | ") : "any"}`;
    case "property":
      return c.value != null && c.value !== "" ? `${c.key}: ${c.value}` : `${c.key}: any`;
    case "scheduled":
      return "scheduled";
    case "deadline":
      return "deadline";
    case "journal":
      return "on journal page";
    case "between": {
      const f = c.field && c.field !== "any" ? `${c.field} ` : "";
      return `${f}between: ${c.start || "?"} ~ ${c.end || "?"}`;
    }
    case "onPage":
      return `page: ${c.name}`;
    case "namespace":
      return `namespace: ${c.ns}`;
    case "pageProperty":
      return c.value != null && c.value !== "" ? `page ${c.key}: ${c.value}` : `page ${c.key}: any`;
    case "pageTags":
      return `page tags: ${c.tags.join(" | ")}`;
    case "content":
      return `text: "${c.text}"`;
    case "sortBy":
      return `sort: ${sortLabel(c.field, c.dir)}`;
    case "aggregate":
      return c.agg === "count" ? "count" : `${c.agg} of ${c.field ?? "?"}`;
    case "groupBy":
      return `group by ${c.field}`;
    case "raw":
      return c.text;
    case "op":
      return c.op.toUpperCase();
  }
}

// ---------------------------------------------------------------------------
// Immutable tree mutations. `loc` is a path of child indices from the root op.
// `[]` denotes the root itself.
// ---------------------------------------------------------------------------

function clone(root: Clause): Clause {
  return structuredClone(root);
}

/** Resolve `loc` to the op node that *contains* the addressed clause and the
 *  index within it. Returns null for the root or an invalid path. */
function locate(root: Clause, loc: number[]): { parent: Clause & { kind: "op" }; idx: number } | null {
  if (loc.length === 0) return null;
  let node: Clause = root;
  for (let i = 0; i < loc.length - 1; i++) {
    if (node.kind !== "op") return null;
    node = node.children[loc[i]];
    if (!node) return null;
  }
  if (node.kind !== "op") return null;
  return { parent: node, idx: loc[loc.length - 1] };
}

/** Resolve `loc` to the op node it addresses ([] = root). */
function opAt(root: Clause, loc: number[]): (Clause & { kind: "op" }) | null {
  let node: Clause = root;
  for (const i of loc) {
    if (node.kind !== "op") return null;
    node = node.children[i];
    if (!node) return null;
  }
  return node.kind === "op" ? node : null;
}

/** Drop empty `and`/`or` nodes (no children) anywhere except the root, so a
 *  query never serializes to a vacuous `(and)` that would match everything. */
function prune(c: Clause): Clause | null {
  if (c.kind !== "op") return c;
  if (c.op === "not") {
    const child = c.children[0] ? prune(c.children[0]) : null;
    return child ? { kind: "op", op: "not", children: [child] } : null;
  }
  const kids = c.children.map(prune).filter((x): x is Clause => x != null);
  if (kids.length === 0) return null;
  return { kind: "op", op: c.op, children: kids };
}

function normalize(root: Clause): Clause {
  if (root.kind !== "op") return root;
  const kids = root.children.map(prune).filter((x): x is Clause => x != null);
  return { kind: "op", op: root.op === "not" ? "and" : root.op, children: kids };
}

/** Append `clause` to the op addressed by `opLoc` ([] = root). */
export function addChild(root: Clause, opLoc: number[], clause: Clause): Clause {
  const r = clone(root);
  const target = opAt(r, opLoc);
  if (!target) return root;
  target.children.push(clause);
  return normalize(r);
}

export function removeAt(root: Clause, loc: number[]): Clause {
  const r = clone(root);
  const at = locate(r, loc);
  if (!at) return root;
  at.parent.children.splice(at.idx, 1);
  return normalize(r);
}

export function replaceAt(root: Clause, loc: number[], clause: Clause): Clause {
  const r = clone(root);
  const at = locate(r, loc);
  if (!at) return root;
  at.parent.children[at.idx] = clause;
  return normalize(r);
}

/** Wrap the clause at `loc` in a new operator node. */
export function wrapAt(root: Clause, loc: number[], op: "and" | "or" | "not"): Clause {
  const r = clone(root);
  const at = locate(r, loc);
  if (!at) return root;
  const cur = at.parent.children[at.idx];
  at.parent.children[at.idx] = { kind: "op", op, children: [cur] };
  return normalize(r);
}

/** Replace the op node at `loc` with its children spliced into the parent. */
export function unwrapAt(root: Clause, loc: number[]): Clause {
  const r = clone(root);
  const at = locate(r, loc);
  if (!at) return root;
  const cur = at.parent.children[at.idx];
  if (cur.kind !== "op") return root;
  at.parent.children.splice(at.idx, 1, ...cur.children);
  return normalize(r);
}

/** Change the operator of the op node addressed by `loc` ([] = root). */
export function setOp(root: Clause, loc: number[], op: "and" | "or"): Clause {
  const r = clone(root);
  const node = opAt(r, loc);
  if (!node) return root;
  node.op = op;
  return normalize(r);
}
