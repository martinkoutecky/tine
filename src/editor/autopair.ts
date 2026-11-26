// Optional bracket/quote auto-pairing for the block editor (a Tine convenience,
// OFF by default — see `autoPairing` in ../ui). Pure text ops: no DOM, so the
// logic is unit-tested here and Block.tsx just applies the {value,caret} / Edit.
//
// This COMPOSES with the always-on OG-style page-ref pairing (`autoPairEdit` in
// ./autocomplete, which auto-closes `[[`→`[[]]` and types-through a manual `]`):
// when auto-pairing is on, `autoPairInsertOnInput` below supersedes it and also
// handles `[`/`(`/`{` doubling, so typing `[` then `[` still yields `[[|]]` (not
// `[[]|]`). See the DOUBLE-open branch.
//
// Scope of the pair set: brackets/parens/braces, plus the symmetric `"` and
// backtick. Emphasis chars (`*` `_` `=`) are deliberately EXCLUDED — pairing them
// fights ordinary Markdown typing (a line-leading `*` is a bullet; `==` etc.) and
// bold/italic/highlight already have the toolbar + Ctrl shortcuts. Apostrophe is
// excluded too (it would fight contractions like "don't").

import type { Edit } from "./format";

/** Opener → closer. Symmetric pairs (`"`, backtick) close with themselves. */
export const PAIRS: Record<string, string> = {
  "(": ")",
  "[": "]",
  "{": "}",
  '"': '"',
  "`": "`",
};

/** Brackets that ALSO double-open (`[[`, `((`, `{{`). Quotes/backtick don't. */
const DOUBLE: Record<string, string> = { "[": "]", "(": ")", "{": "}" };

/** Openers that WRAP A SELECTION when typed (opener → closer), matching OG's
 *  `autopair-map`. This is a SUPERSET of `PAIRS`: it adds the emphasis marks
 *  (`* _ ^ = ~ / +`), which OG pairs *only* when text is selected (never on an
 *  empty caret, where they'd fight ordinary Markdown/bullet typing) — so `*`/`~`/
 *  `=` wrap a selection (and double to `**bold**`/`~~strike~~`/`==highlight==`),
 *  and `/`/`+`/`_`/`^` cover the Org emphasis markers. `[`/`(` additionally open
 *  the page/block search on their doubling bracket (see `doubleRefKind`). `"` is a
 *  Tine extra (OG doesn't pair it) kept because it's harmless and useful. Selection
 *  wrapping is ALWAYS-ON, independent of the opt-in empty-caret auto-pairing. */
export const SELECTION_WRAP: Record<string, string> = {
  "（": ")",
  "【": "]",
  "(": ")",
  "[": "]",
  "{": "}",
  '"': '"',
  "`": "`",
  "~": "~",
  "*": "*",
  "_": "_",
  "^": "^",
  "=": "=",
  "/": "/",
  "+": "+",
};

/** The closer characters (values of PAIRS), for skip-over / before-close tests. */
export const CLOSERS = new Set(Object.values(PAIRS));

/** Only auto-close before a boundary: end-of-text, whitespace, or a closer. This
 *  is the standard editor heuristic that avoids inserting a stray closer when you
 *  type an opener in the middle of a word (`(` before `word` → just `(word`). */
function shouldAutoClose(next: string | undefined): boolean {
  return !next || /\s/.test(next) || CLOSERS.has(next);
}

/** A quote or backtick typed after a letter or a number (with its combining marks)
 *  closes what an earlier one opened (`"hello` + `"`), so it gets no partner
 *  (GH #1020). So does each further one typed directly after it: the word is looked
 *  for before the whole run of that delimiter, so two closing backticks stay two.
 *  `at` is the index of the typed character. Brackets have a distinct opener and
 *  never hit this. CodeMirror and VS Code have a rule of the same kind: no new
 *  string starts after a word character. Reads the run plus at most 8 UTF-16
 *  units, which covers a letter outside the BMP and its marks. */
function closesAfterWord(typed: string, value: string, at: number): boolean {
  if (PAIRS[typed] !== typed) return false;
  let start = at;
  while (start > 0 && value[start - 1] === typed) start--;
  return /[\p{L}\p{N}]\p{M}*$/u.test(value.slice(Math.max(0, start - 8), start));
}

/** Post-input auto-pairing (run from `onInput`, AFTER the browser inserted the
 *  single char `typed`). Returns the adjusted `{value, caret}` or null when
 *  nothing should change. Handles, in order:
 *   1. double-open upgrade — the 2nd bracket of `[[`/`((`/`{{` closes the pair
 *      (`[[|]]`), upgrading a single auto-paired closer if one already follows;
 *   2. skip-over — typing a closer that already sits right after the caret steps
 *      over it instead of stacking a duplicate;
 *   3. single pair insert — typing an opener at a boundary inserts its closer,
 *      caret between; a quote or backtick right after a letter or number is a
 *      closer and inserts nothing. */
export function autoPairInsertOnInput(
  value: string,
  caret: number,
  typed: string
): { value: string; caret: number } | null {
  // 1) Double-open: typing the second `[`/`(`/`{` of a pair.
  if (typed in DOUBLE && caret >= 2 && value[caret - 2] === typed && value[caret - 1] === typed) {
    const cl = DOUBLE[typed];
    // A single closer already sits after the caret (from the first bracket's own
    // pairing) → add one more so `[[]` becomes `[[]]`. Otherwise add the full pair.
    const add = value[caret] === cl ? cl : cl + cl;
    return { value: value.slice(0, caret) + add + value.slice(caret), caret };
  }
  // 2) Skip-over: closer typed right before the identical closer.
  if (CLOSERS.has(typed) && value[caret] === typed) {
    return { value: value.slice(0, caret - 1) + value.slice(caret), caret };
  }
  // 3) Single pair insert.
  const closer = PAIRS[typed];
  if (closer && shouldAutoClose(value[caret]) && !closesAfterWord(typed, value, caret - 1)) {
    return { value: value.slice(0, caret) + closer + value.slice(caret), caret };
  }
  return null;
}

/** Typing an opener with a NON-empty selection wraps the selection in the pair
 *  (`(sel)`), keeping the selection around the inner text. Uses `SELECTION_WRAP`
 *  (the OG wrap set, incl. emphasis marks), NOT the empty-caret `PAIRS`. Returns
 *  null for a non-wrap key or an empty selection. */
export function wrapSelectionEdit(text: string, start: number, end: number, opener: string): Edit | null {
  // OG keycode's full-width aliases wrap with the same ASCII delimiters.
  opener = opener === "（" ? "(" : opener === "【" ? "[" : opener;
  const closer = SELECTION_WRAP[opener];
  if (!closer || start === end) return null;
  const sel = text.slice(start, end);
  return {
    text: text.slice(0, start) + opener + sel + closer + text.slice(end),
    start: start + opener.length,
    end: end + opener.length,
  };
}

/** After a selection is wrapped by `[` or `(`, report whether the inner (still-
 *  selected) text `[innerStart,innerEnd)` is now enclosed by a DOUBLE bracket —
 *  `[[x]]` (page ref) or `((x))` (block ref). That is the SECOND bracket of a
 *  `[[`/`((`, the point where OG opens the page/block search seeded with the
 *  selection (`:editor/search-page` / `:editor/search-block`). Returns null on the
 *  first bracket (`[x]`) or a non-ref pair (`{`, `"`, backtick). */
export function doubleRefKind(text: string, innerStart: number, innerEnd: number): "page" | "block" | null {
  if (innerStart < 2) return null;
  if (text.slice(innerStart - 2, innerStart) === "[[" && text.slice(innerEnd, innerEnd + 2) === "]]") return "page";
  if (text.slice(innerStart - 2, innerStart) === "((" && text.slice(innerEnd, innerEnd + 2) === "))") return "block";
  return null;
}

/** Backspace with the caret BETWEEN an empty pair (`(|)`, `"|"`, …) deletes BOTH
 *  chars. Returns null when the caret isn't between a matching empty pair. */
export function backspacePairEdit(text: string, caret: number): Edit | null {
  const prev = text[caret - 1];
  const next = text[caret];
  if (prev && next && PAIRS[prev] === next) {
    return { text: text.slice(0, caret - 1) + text.slice(caret + 1), start: caret - 1, end: caret - 1 };
  }
  return null;
}
