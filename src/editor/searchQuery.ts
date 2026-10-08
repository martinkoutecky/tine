// Friendly search metadata, membership and evidence come from tine-search via wasm.
// Initialize the app parser before calling this synchronous door. Parsing costs
// O(query bytes + bounded compilation); membership O(text × terms); evidence
// O(text × needle scalars), capped by limit. Invalid programs match nothing.
import { search_query_json, search_matches, search_spans_json, search_substring_spans_json } from "../render/wasm/lsdoc_wasm.js";

interface Term { text: string; negated: boolean; quoted: boolean }
interface SearchSource { query: string; removeAccents: boolean; simple: string | null }
export type SearchMatcher = SearchSource & (
  | { kind: "empty" }
  | { kind: "invalid"; error: string }
  | { kind: "regex"; pattern: string }
  | { kind: "boolean"; groups: Term[][] }
);
interface SourceSpan { start: number; end: number }

function wasmLimit(limit: number): number {
  return Math.min(0xffffffff, Math.max(0, Math.floor(limit) || 0));
}

/** Shared folded substring evidence in original UTF-16 coordinates. */
export function searchSubstringSpans(text: string, needle: string, limit = Number.POSITIVE_INFINITY, removeAccents = true): SourceSpan[] {
  return JSON.parse(search_substring_spans_json(text, needle, wasmLimit(limit), removeAccents)) as SourceSpan[];
}

/** Native grammar and bounded Rust regex validation; source is retained so
 * parser recovery/cache eviction can transparently recompile the matcher. */
export function parseSearchQuery(query: string, removeAccents = true): SearchMatcher {
  return { ...JSON.parse(search_query_json(query, removeAccents)), query, removeAccents } as SearchMatcher;
}

/** lower must use the matcher's fold policy; regex sees only original text. */
export function matcherMatches(m: SearchMatcher, lower: string, orig: string): boolean {
  return search_matches(m.query, m.removeAccents, lower, orig);
}

export function simpleTerm(m: SearchMatcher): string | null {
  return m.simple;
}

/** Earliest positive evidence; zero-width regex hits retain their position. */
export function matchHighlight(m: SearchMatcher, text: string): { start: number; len: number } | null {
  const span = (JSON.parse(search_spans_json(m.query, m.removeAccents, text, 1, true)) as SourceSpan[])[0];
  return span ? { start: span.start, len: span.end - span.start } : null;
}

/** Positive evidence for the first matching group, with empty regex hits omitted. */
export function matchHighlights(m: SearchMatcher, text: string, limit = 24): SourceSpan[] {
  return JSON.parse(search_spans_json(m.query, m.removeAccents, text, wasmLimit(limit), false)) as SourceSpan[];
}

export const SEARCH_SYNTAX = [
  { example: "foo bar", description: "contains both terms", match: "bar then foo", miss: "foo only" },
  { example: "foo OR bar", description: "contains either term", match: "bar only", miss: "neither" },
  { example: "foo -draft", description: "contains foo, excludes draft", match: "foo ready", miss: "foo draft" },
  { example: '"exact phrase"', description: "matches adjacent words", match: "an exact phrase here", miss: "exact other phrase" },
  { example: "/[A-Z]{3}/", description: "case-sensitive regular expression", match: "ABC", miss: "abc" },
] as const;

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
    return { dsl: `(content-regex ${quoteDsl(matcher.pattern)})`, error: null };
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
 * The `(search …)` predicate is a Tine query extension evaluated by the
 * in-memory query evaluator using the graph's search policy. */
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
