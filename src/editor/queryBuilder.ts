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

export type Clause =
  | { kind: "op"; op: "and" | "or" | "not"; children: Clause[] }
  | { kind: "page"; name: string } // a [[page]] / #tag / (page-ref) reference
  | { kind: "task"; markers: string[] }
  | { kind: "priority"; levels: string[] }
  | { kind: "property"; key: string; value: string | null }
  | { kind: "scheduled" }
  | { kind: "deadline" }
  | { kind: "between"; start: string; end: string }
  | { kind: "onPage"; name: string } // (page name) — blocks on a named page
  | { kind: "namespace"; ns: string }
  | { kind: "pageProperty"; key: string; value: string | null }
  | { kind: "pageTags"; tags: string[] }
  | { kind: "content"; text: string }
  // Verbatim fallback for a sub-expression we don't model, so an unfamiliar
  // (but non-datalog) query round-trips losslessly instead of being discarded.
  | { kind: "raw"; text: string };

export const MARKERS = ["TODO", "DOING", "NOW", "LATER", "DONE", "WAITING", "CANCELED"];
export const PRIORITIES = ["A", "B", "C"];

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
      while (j < ch.length && ch[j] !== '"') s += ch[j++];
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
        const value = parseOptName(toks, cur);
        clause = { kind: "property", key, value };
        break;
      }
      case "page-property": {
        const key = parseName(toks, cur);
        if (key == null) {
          clause = null;
          break;
        }
        const value = parseOptName(toks, cur);
        clause = { kind: "pageProperty", key, value };
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
      case "between": {
        const start = parseName(toks, cur) ?? "";
        const end = parseName(toks, cur) ?? "";
        clause = { kind: "between", start, end };
        break;
      }
      default:
        clause = null;
    }
    // Consume up to and including the matching ")".
    while (toks[cur.pos] && toks[cur.pos].t !== ")") cur.pos++;
    const close = toks[cur.pos];
    if (close && close.t === ")") cur.pos++;
    if (clause == null && close) {
      // Unknown form: preserve it verbatim so it round-trips.
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

function needsQuote(s: string): boolean {
  return /\s/.test(s) || s === "";
}
function word(s: string): string {
  return needsQuote(s) ? `"${s}"` : s;
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
    case "between":
      return `(between [[${c.start}]] [[${c.end}]])`;
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
      return `"${c.text}"`;
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
    case "between":
      return `between: ${c.start || "?"} ~ ${c.end || "?"}`;
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
