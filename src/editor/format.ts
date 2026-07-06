// Pure text operations for the block editor: inline-format toggles, link
// insertion, and Emacs-style cursor/kill motions. No DOM — each function takes
// the raw text + selection and returns the new text + selection, so Block.tsx
// just applies the result to the textarea. Unit-testable.

import { MARKER_RE } from "../markers";

export interface Edit {
  text: string;
  start: number;
  end: number;
}

/** Toggle a symmetric inline wrap (e.g. `**` … `**`). Unwraps if the selection
 *  is already wrapped (markers just outside, or included in the selection).
 *  With an empty selection, inserts the pair and places the caret between. */
export function toggleWrap(text: string, start: number, end: number, left: string, right = left): Edit {
  const sel = text.slice(start, end);

  // Markers immediately outside the selection -> unwrap.
  if (
    start >= left.length &&
    text.slice(start - left.length, start) === left &&
    text.slice(end, end + right.length) === right
  ) {
    return {
      text: text.slice(0, start - left.length) + sel + text.slice(end + right.length),
      start: start - left.length,
      end: end - left.length,
    };
  }

  // Markers included in the selection -> unwrap inner.
  if (sel.startsWith(left) && sel.endsWith(right) && sel.length >= left.length + right.length) {
    const inner = sel.slice(left.length, sel.length - right.length);
    return { text: text.slice(0, start) + inner + text.slice(end), start, end: start + inner.length };
  }

  // Empty selection -> insert pair, caret between.
  if (start === end) {
    return { text: text.slice(0, start) + left + right + text.slice(end), start: start + left.length, end: start + left.length };
  }

  // Wrap the selection.
  return {
    text: text.slice(0, start) + left + sel + right + text.slice(end),
    start: start + left.length,
    end: end + left.length,
  };
}

/** Insert a markdown link. With a selection, it becomes the label and the caret
 *  lands inside the (url); with no selection, inserts `[]()` caret in the label. */
export function insertLink(text: string, start: number, end: number): Edit {
  const sel = text.slice(start, end);
  if (sel) {
    const out = `[${sel}](`;
    const next = text.slice(0, start) + out + ")" + text.slice(end);
    const caret = start + out.length; // inside ()
    return { text: next, start: caret, end: caret };
  }
  const next = text.slice(0, start) + "[]()" + text.slice(end);
  return { text: next, start: start + 1, end: start + 1 }; // caret in []
}

// A single bare URL (one token, known scheme, no whitespace). Used by the
// "paste a URL over a selection → link the selection" behavior (#23). We key on
// an explicit scheme — `http(s)://` and `mailto:` cover the real cases — and
// deliberately exclude scheme-less `www.…` (ambiguous, and OG's autolinker
// keys on a scheme too). Trim before calling.
const PASTE_URL_RE = /^(?:https?:\/\/|mailto:)\S+$/i;
export function isPasteableUrl(text: string): boolean {
  return PASTE_URL_RE.test(text.trim());
}

/** Wrap a selection as a link around a pasted `url`. Format-aware: markdown
 *  `[sel](url)`, org `[[url][sel]]` (org puts the target first, label second —
 *  the inverse of markdown). The caret lands just after the inserted link. */
export function wrapLink(
  text: string,
  start: number,
  end: number,
  url: string,
  format: "md" | "org",
): Edit {
  const sel = text.slice(start, end);
  const link = format === "org" ? `[[${url}][${sel}]]` : `[${sel}](${url})`;
  const next = text.slice(0, start) + link + text.slice(end);
  const caret = start + link.length;
  return { text: next, start: caret, end: caret };
}

// --- line helpers (a "line" is bounded by \n or the text ends) ---
function lineStart(text: string, pos: number): number {
  const nl = text.lastIndexOf("\n", pos - 1);
  return nl === -1 ? 0 : nl + 1;
}
function lineEnd(text: string, pos: number): number {
  const nl = text.indexOf("\n", pos);
  return nl === -1 ? text.length : nl;
}

/** Emacs Ctrl+U: delete from line start to caret. */
export function killLineBefore(text: string, caret: number): Edit {
  const ls = lineStart(text, caret);
  return { text: text.slice(0, ls) + text.slice(caret), start: ls, end: ls };
}

/** Emacs Ctrl+K / Alt+K: delete from caret to line end. */
export function killLineAfter(text: string, caret: number): Edit {
  const le = lineEnd(text, caret);
  return { text: text.slice(0, caret) + text.slice(le), start: caret, end: caret };
}

const WORD = /[A-Za-z0-9_]/;
/** Next word boundary at or after `caret`. */
export function wordForward(text: string, caret: number): number {
  let i = caret;
  while (i < text.length && !WORD.test(text[i])) i++;
  while (i < text.length && WORD.test(text[i])) i++;
  return i;
}
/** Previous word boundary at or before `caret`. */
export function wordBackward(text: string, caret: number): number {
  let i = caret;
  while (i > 0 && !WORD.test(text[i - 1])) i--;
  while (i > 0 && WORD.test(text[i - 1])) i--;
  return i;
}
export function killWordForward(text: string, caret: number): Edit {
  const to = wordForward(text, caret);
  return { text: text.slice(0, caret) + text.slice(to), start: caret, end: caret };
}
export function killWordBackward(text: string, caret: number): Edit {
  const from = wordBackward(text, caret);
  return { text: text.slice(0, from) + text.slice(caret), start: from, end: from };
}

// --- priority (sets/replaces `[#A]` after a leading task marker) ---
// MARKER_RE is the shared anchor (src/markers.ts).

/** Set (or replace) the `[#X]` priority on a block's first line, placed after
 *  any task marker. Mirrors OG's add-or-update-priority. */
export function setPriority(firstLine: string, level: "A" | "B" | "C"): string {
  const m = MARKER_RE.exec(firstLine);
  const markerEnd = m ? m[0].length : 0;
  const head = firstLine.slice(0, markerEnd); // "TODO" or ""
  let rest = firstLine.slice(markerEnd).replace(/^\s+/, ""); // body after marker
  rest = rest.replace(/^\[#[ABC]\]\s*/, ""); // drop an existing priority token
  const prefix = head ? `${head} ` : "";
  return rest ? `${prefix}[#${level}] ${rest}` : `${prefix}[#${level}]`;
}

// A trimmed last line that is JUST an (empty) in-block list-item prefix — a bare
// `*`/`+`/`-`/`1.`/`1)` marker, optionally followed by an empty `[ ]`/`[x]`
// checkbox. Its trailing space is syntactically required (the list/checkbox
// renderers need whitespace after the marker), so it must NOT be trimmed away.
const LIST_ITEM_PREFIX_RE = /^\s*(?:[-+*]|\d+[.)])(?:\s+\[[ xX]\])?$/;

/** Trailing spaces/tabs at the very end of a block's visible text are an editing
 *  convenience only — e.g. the space left after a `/priority` insert so the next
 *  word or `/command` flows without manually adding one (the slash menu needs a
 *  whitespace boundary before `/`). Never persist them, matching OG, which trims
 *  the block on save. Only the absolute end is trimmed (not internal lines, not
 *  leading indent, not a trailing newline), so list continuation lines and code
 *  blocks are untouched — and an empty trailing list/checkbox item keeps the one
 *  space its marker needs. */
export function trimBlockTrailingSpace(text: string): string {
  const trimmed = text.replace(/[ \t]+$/, "");
  if (trimmed === text) return text;
  const lastLine = trimmed.slice(trimmed.lastIndexOf("\n") + 1);
  return LIST_ITEM_PREFIX_RE.test(lastLine) ? `${trimmed} ` : trimmed;
}
