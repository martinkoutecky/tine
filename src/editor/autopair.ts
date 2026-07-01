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

/** The closer characters (values of PAIRS), for skip-over / before-close tests. */
export const CLOSERS = new Set(Object.values(PAIRS));

/** Only auto-close before a boundary: end-of-text, whitespace, or a closer. This
 *  is the standard editor heuristic that avoids inserting a stray closer when you
 *  type an opener in the middle of a word (`(` before `word` → just `(word`). */
function shouldAutoClose(next: string | undefined): boolean {
  return !next || /\s/.test(next) || CLOSERS.has(next);
}

/** Post-input auto-pairing (run from `onInput`, AFTER the browser inserted the
 *  single char `typed`). Returns the adjusted `{value, caret}` or null when
 *  nothing should change. Handles, in order:
 *   1. double-open upgrade — the 2nd bracket of `[[`/`((`/`{{` closes the pair
 *      (`[[|]]`), upgrading a single auto-paired closer if one already follows;
 *   2. skip-over — typing a closer that already sits right after the caret steps
 *      over it instead of stacking a duplicate;
 *   3. single pair insert — typing an opener at a boundary inserts its closer,
 *      caret between. */
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
  if (closer && shouldAutoClose(value[caret])) {
    return { value: value.slice(0, caret) + closer + value.slice(caret), caret };
  }
  return null;
}

/** Typing an opener with a NON-empty selection wraps the selection in the pair
 *  (`(sel)`), keeping the selection around the inner text. Returns null for a
 *  non-opener or an empty selection (the caller falls through to insert-pair). */
export function wrapSelectionEdit(text: string, start: number, end: number, opener: string): Edit | null {
  const closer = PAIRS[opener];
  if (!closer || start === end) return null;
  const sel = text.slice(start, end);
  return {
    text: text.slice(0, start) + opener + sel + closer + text.slice(end),
    start: start + opener.length,
    end: end + opener.length,
  };
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
