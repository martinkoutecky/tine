// Pure text operations for the block editor: inline-format toggles, link
// insertion, and Emacs-style cursor/kill motions. No DOM — each function takes
// the raw text + selection and returns the new text + selection, so Block.tsx
// just applies the result to the textarea. Unit-testable.

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
const MARKER_RE = /^(NOW|LATER|TODO|DOING|DONE|WAITING|WAIT|CANCELED|CANCELLED|IN-PROGRESS)\b/;

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
