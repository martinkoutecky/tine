// Shared parser for the Ctrl-K quick-search query dialect (GH #44).
//
// MIRRORS `crates/tine-core/src/search_query.rs` — the grammar, the `is_simple`
// rule, and the match semantics MUST agree with the Rust side, or a query would
// filter the page list (checked here) differently from the block list (checked
// in Rust). See that file's doc comment for the grammar.

export interface Term {
  // Lowercase-plus-NFC needle for `.includes(..)`.
  text: string;
  negated: boolean;
  // Came from a `"quoted phrase"` — an explicit grammar opt-in, so even a single
  // quoted word is not treated as `simple`.
  quoted: boolean;
}

export type SearchMatcher =
  | { kind: "empty" }
  | { kind: "invalid"; error: string }
  | { kind: "regex"; re: RegExp }
  // OR of AND-groups; every retained group has ≥1 positive term.
  | { kind: "boolean"; groups: Term[][] };

export const SEARCH_SYNTAX = [
  { example: "foo bar", description: "contains both terms", match: "bar then foo", miss: "foo only" },
  { example: "foo OR bar", description: "contains either term", match: "bar only", miss: "neither" },
  { example: "foo -draft", description: "contains foo, excludes draft", match: "foo ready", miss: "foo draft" },
  { example: '"exact phrase"', description: "matches adjacent words", match: "an exact phrase here", miss: "exact other phrase" },
  { example: "/[A-Z]{3}/", description: "case-sensitive regular expression", match: "ABC", miss: "abc" },
] as const;

/** Locale-independent comparison form for non-regex search. NFC handles only
 * canonical equivalence: it deliberately does not perform compatibility or
 * accent folding. */
export function canonicalFold(value: string): string {
  return value.toLowerCase().normalize("NFC");
}

interface SourceSpan { start: number; end: number }

const graphemeSegmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/** Fold text while retaining original UTF-16 provenance. Each normalized
 * scalar maps to its complete contributing extended grapheme, so composition,
 * mark reordering, Hangul Jamo, and lowercase expansion cannot leave a partial
 * highlight behind. */
function foldedWithMap(original: string): { scalars: string[]; spans: SourceSpan[] } {
  const lowered = original.toLowerCase();
  const loweredSources: SourceSpan[] = [];
  let originalUtf16 = 0;
  for (const scalar of original) {
    const start = originalUtf16;
    originalUtf16 += scalar.length;
    for (const _ of scalar.toLowerCase()) loweredSources.push({ start, end: originalUtf16 });
  }

  const scalars: string[] = [];
  const spans: SourceSpan[] = [];
  let sourceAt = 0;
  for (const part of graphemeSegmenter.segment(lowered)) {
    const count = Array.from(part.segment).length;
    const contributors = loweredSources.slice(sourceAt, sourceAt + count);
    sourceAt += count;
    const source = {
      start: contributors[0]?.start ?? 0,
      end: contributors.at(-1)?.end ?? 0,
    };
    for (const scalar of part.segment.normalize("NFC")) {
      scalars.push(scalar);
      spans.push(source);
    }
  }
  return { scalars, spans };
}

function canonicalSubstringSpans(text: string, needle: string, limit: number): SourceSpan[] {
  const hay = foldedWithMap(text);
  const wanted = Array.from(canonicalFold(needle));
  if (!wanted.length || wanted.length > hay.scalars.length) return [];
  const out: SourceSpan[] = [];
  for (let at = 0; at <= hay.scalars.length - wanted.length && out.length < limit; at += 1) {
    if (!wanted.every((scalar, index) => hay.scalars[at + index] === scalar)) continue;
    const span = {
      start: hay.spans[at].start,
      end: hay.spans[at + wanted.length - 1].end,
    };
    if (!out.some((existing) => existing.start === span.start && existing.end === span.end)) out.push(span);
  }
  return out;
}

export function parseSearchQuery(query: string): SearchMatcher {
  const q = query.trim();
  if (!q) return { kind: "empty" };
  // Whole-query regex: `/pattern/` with a non-empty pattern. (`//` is too short —
  // an empty pattern matches everything — so it falls through to a literal term.)
  if (q.length >= 3 && q.startsWith("/") && q.endsWith("/")) {
    const pat = q.slice(1, -1);
    try {
      // Case-sensitive (no `i`), matching the Rust `regex` side: the pattern owns
      // its case classes, so `[A-Z]` works.
      return { kind: "regex", re: new RegExp(pat) };
    } catch (e) {
      return { kind: "invalid", error: e instanceof Error ? e.message : "invalid regex" };
    }
  }
  const groups = parseBoolean(q).filter((g) => g.some((t) => !t.negated));
  if (!groups.length) return { kind: "empty" };
  return { kind: "boolean", groups };
}

// Does `orig` (original text) / `lower` (pre-folded lowercase-plus-NFC) match?
export function matcherMatches(m: SearchMatcher, lower: string, orig: string): boolean {
  switch (m.kind) {
    case "regex":
      return m.re.test(orig);
    case "boolean":
      return m.groups.some((g) => groupMatches(g, lower));
    default:
      return false; // empty | invalid
  }
}

// The single positive bare term when this is a one-term query, else null.
export function simpleTerm(m: SearchMatcher): string | null {
  if (m.kind !== "boolean" || m.groups.length !== 1 || m.groups[0].length !== 1) return null;
  const t = m.groups[0][0];
  return !t.negated && !t.quoted ? t.text : null;
}

// The first match range in `text`, for the snippet highlight: the earliest
// positive-term occurrence (boolean) or the first regex match. null if none.
export function matchHighlight(m: SearchMatcher, text: string): { start: number; len: number } | null {
  if (m.kind === "regex") {
    // exec without a global flag returns the first match with its index.
    const hit = m.re.exec(text);
    return hit ? { start: hit.index, len: hit[0].length } : null;
  }
  if (m.kind === "boolean") {
    let best: { start: number; len: number } | null = null;
    for (const g of m.groups) {
      for (const t of g) {
        if (t.negated || !t.text) continue;
        const span = canonicalSubstringSpans(text, t.text, 1)[0];
        if (span && (!best || span.start < best.start)) {
          best = { start: span.start, len: span.end - span.start };
        }
      }
    }
    return best;
  }
  return null;
}

/** All positive match ranges for dev/mock presentation only. Production search
 * receives authoritative UTF-16 evidence from Rust's QueryPlan evaluator. */
export function matchHighlights(m: SearchMatcher, text: string, limit = 24): { start: number; end: number }[] {
  if (m.kind === "regex") {
    const flags = m.re.flags.includes("g") ? m.re.flags : `${m.re.flags}g`;
    const re = new RegExp(m.re.source, flags);
    const out: { start: number; end: number }[] = [];
    for (const hit of text.matchAll(re)) {
      const start = hit.index ?? 0;
      const end = start + hit[0].length;
      if (end > start) out.push({ start, end });
      if (out.length >= limit) break;
    }
    return out;
  }
  if (m.kind !== "boolean") return [];
  const lower = canonicalFold(text);
  const group = m.groups.find((candidate) => groupMatches(candidate, lower));
  if (!group) return [];
  const out: { start: number; end: number }[] = [];
  for (const term of group) {
    if (term.negated || !term.text) continue;
    out.push(...canonicalSubstringSpans(text, term.text, limit - out.length));
  }
  return out.sort((a, b) => a.start - b.start || a.end - b.end);
}

function quoteDsl(value: string): string {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

/** Lossless compiler for the friendly block-search grammar into the ordinary
 * simple query DSL. Page fuzzy matching is intentionally not implied here: it
 * is an explicit page branch in QueryPlan, while this compiler is used when a
 * user deliberately switches a workspace to the block-query builder. */
export function friendlySearchToDsl(query: string): { dsl: string; error: string | null } {
  const matcher = parseSearchQuery(query);
  if (matcher.kind === "invalid") return { dsl: "", error: matcher.error };
  if (matcher.kind === "empty") return { dsl: "", error: "Add at least one positive search term." };
  if (matcher.kind === "regex") {
    return { dsl: `(content-regex ${quoteDsl(matcher.re.source)})`, error: null };
  }
  const termDsl = (term: Term) => {
    const content = quoteDsl(term.text);
    return term.negated ? `(not ${content})` : content;
  };
  const groups = matcher.groups.map((group) => {
    const terms = group.map(termDsl);
    return terms.length === 1 ? terms[0] : `(and ${terms.join(" ")})`;
  });
  return { dsl: groups.length === 1 ? groups[0] : `(or ${groups.join(" ")})`, error: null };
}

/** Canonical lossless on-disk representation for a friendly search workspace.
 * The `(search …)` predicate is a Tine query extension compiled by the same
 * Rust QueryPlan as Ctrl+K; it keeps the friendly source reconstructible. */
export function friendlySearchToSavedDsl(query: string): string {
  return `(search ${quoteDsl(query.trim())})`;
}

/** Recover friendly source from the canonical `(search "…")` query extension.
 * Returns null for any other DSL so frontends never pretend a lossy conversion
 * is reversible. */
export function savedDslToFriendlySearch(dsl: string): string | null {
  const match = /^\(\s*search\s+"((?:[^"\\]|\\.)*)"\s*\)$/s.exec(dsl.trim());
  if (!match) return null;
  let out = "";
  for (let i = 0; i < match[1].length; i += 1) {
    const char = match[1][i];
    if (char === "\\" && i + 1 < match[1].length && (match[1][i + 1] === "\\" || match[1][i + 1] === '"')) {
      out += match[1][i + 1];
      i += 1;
    } else out += char;
  }
  return out;
}

function groupMatches(group: Term[], lower: string): boolean {
  return group.every((t) => {
    const present = t.text !== "" && lower.includes(t.text);
    return present !== t.negated;
  });
}

function parseBoolean(q: string): Term[][] {
  const tokens = tokenize(q);
  const groups: Term[][] = [];
  let cur: Term[] = [];
  for (const tok of tokens) {
    if (tok.isOr) {
      groups.push(cur);
      cur = [];
      continue;
    }
    if (!tok.text) continue;
    cur.push({ text: canonicalFold(tok.text), negated: tok.negated, quoted: tok.quoted });
  }
  groups.push(cur);
  return groups.filter((g) => g.length > 0);
}

interface Token {
  text: string;
  negated: boolean;
  quoted: boolean;
  isOr: boolean;
}

// Split into tokens, honoring `"quoted phrases"` (may contain spaces) and a
// leading `-` for negation. A bare unquoted `OR` becomes an OR separator.
function tokenize(q: string): Token[] {
  const chars = Array.from(q);
  const out: Token[] = [];
  let i = 0;
  while (i < chars.length) {
    if (/\s/.test(chars[i])) {
      i += 1;
      continue;
    }
    let negated = false;
    // Leading `-` negates, but only when something non-space follows it.
    if (chars[i] === "-" && i + 1 < chars.length && !/\s/.test(chars[i + 1])) {
      negated = true;
      i += 1;
    }
    let text: string;
    let quoted: boolean;
    if (i < chars.length && chars[i] === '"') {
      // Quoted phrase: read to the closing quote (or end of input).
      i += 1;
      const start = i;
      while (i < chars.length && chars[i] !== '"') i += 1;
      text = chars.slice(start, i).join("");
      if (i < chars.length) i += 1; // consume closing quote
      quoted = true;
    } else {
      // Bare token: read to the next whitespace.
      const start = i;
      while (i < chars.length && !/\s/.test(chars[i])) i += 1;
      text = chars.slice(start, i).join("");
      quoted = false;
    }
    if (!quoted && !negated && text === "OR") {
      out.push({ text: "", negated: false, quoted: false, isOr: true });
    } else {
      out.push({ text, negated, quoted, isOr: false });
    }
  }
  return out;
}
