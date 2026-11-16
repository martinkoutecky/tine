// Browser-preview twin of the `((` block picker's two OG legs (the native
// engine is crates/tine-store `query_plan/words.rs`; ADR 0016 keeps this a
// preview approximation). OG's electron `search-blocks` lists
//   1. FTS5 `content MATCH` hits: the typed words as whole tokens, any order;
//   2. `content LIKE '%a%b%'` hits: the typed pieces as literal substrings, in order;
// de-duplicated, first leg first. Inputs are already search-folded.

const TOKENS = /[\p{L}\p{N}\p{M}\p{Co}]+/gu;
// What an FTS5 query accepts as a bare word: ASCII alphanumerics, `_`, and any
// non-ASCII character. Anything else (`-`, `'`, `:`, `"`, `.`, ...) is a syntax
// error for the unquoted MATCH, and OG then only runs its quoted-phrase input.
const BAREWORD = /^[A-Za-z0-9_\u0080-\u{10ffff}]+$/u;

function tokens(text: string): string[] {
  return text.match(TOKENS) ?? [];
}

/** The word groups of the full-text leg: each group is a phrase (adjacent
 *  tokens, in order); every group must occur. Empty means the leg is off. */
export function blockPickerWordGroups(foldedQuery: string): string[][] {
  const pieces = foldedQuery.split(/\s+/).filter(Boolean);
  if (pieces.length === 0) return [];
  const groups = pieces.every((piece) => BAREWORD.test(piece))
    ? pieces.map(tokens)
    : [tokens(foldedQuery)];
  return groups.filter((group) => group.length > 0);
}

function hasPhrase(hay: string[], phrase: string[]): boolean {
  for (let i = 0; i + phrase.length <= hay.length; i++) {
    if (phrase.every((word, k) => hay[i + k] === word)) return true;
  }
  return false;
}

function fragmentsInOrder(text: string, foldedQuery: string): boolean {
  let at = 0;
  let any = false;
  for (const fragment of foldedQuery.split(/\s+/)) {
    if (!fragment) continue;
    const found = text.indexOf(fragment, at);
    if (found < 0) return false;
    at = found + fragment.length;
    any = true;
  }
  return any;
}

/** Which OG leg lists this block, the full-text one winning when both do. */
export function blockPickerLeg(foldedText: string, foldedQuery: string): "words" | "fragments" | null {
  const groups = blockPickerWordGroups(foldedQuery);
  if (groups.length > 0) {
    const hay = tokens(foldedText);
    if (groups.every((group) => hasPhrase(hay, group))) return "words";
  }
  return fragmentsInOrder(foldedText, foldedQuery) ? "fragments" : null;
}

/** Merge the two legs OG's way: words first, then fragment-only, cut at `limit`. */
export function mergeBlockPickerLegs<T extends { blocks: unknown[] }>(
  words: T[],
  fragments: T[],
  limit: number,
): T[] {
  const out: T[] = [];
  let budget = limit;
  for (const group of [...words, ...fragments]) {
    if (budget <= 0) break;
    const blocks = group.blocks.slice(0, budget);
    budget -= blocks.length;
    if (blocks.length > 0) out.push({ ...group, blocks });
  }
  return out;
}
