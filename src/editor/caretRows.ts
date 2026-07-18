// Visual-row detection for a <textarea>. Vertical caret navigation (Arrow and
// Shift+Arrow) must move WITHIN a block's wrapped text and only cross to an
// adjacent block when the caret is on the first/last VISUAL row. A source-`\n`
// test isn't enough: one long line with no newline still wraps into several
// visual rows, and treating it as a single line makes Up/Down (and shift-select)
// jump to the neighbouring block from the middle of a wrapped line.
//
// We measure by mirroring the textarea into an off-screen div with identical
// wrapping metrics and reading the offsetTop of a marker placed at the caret.
// Where there is no real layout (jsdom/tests), offsetHeight is 0; we then return
// `true` so callers fall back to their cheap source-`\n` heuristic (old behaviour)
// rather than misfiring.

const COPY_PROPS = [
  "boxSizing",
  "width",
  "paddingTop",
  "paddingRight",
  "paddingBottom",
  "paddingLeft",
  "borderTopWidth",
  "borderRightWidth",
  "borderBottomWidth",
  "borderLeftWidth",
  "fontFamily",
  "fontSize",
  "fontWeight",
  "fontStyle",
  "fontVariant",
  "letterSpacing",
  "lineHeight",
  "textTransform",
  "textIndent",
  "wordSpacing",
  "tabSize",
] as const;

function buildMirror(ta: HTMLTextAreaElement): HTMLDivElement {
  const div = document.createElement("div");
  const cs = getComputedStyle(ta);
  for (const p of COPY_PROPS) (div.style as unknown as Record<string, string>)[p] = cs.getPropertyValue(camelToKebab(p));
  div.style.position = "absolute";
  div.style.top = "0";
  div.style.left = "-9999px";
  div.style.visibility = "hidden";
  div.style.whiteSpace = "pre-wrap";
  div.style.wordWrap = "break-word";
  div.style.overflowWrap = "break-word";
  return div;
}

function camelToKebab(s: string): string {
  return s.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase());
}

// Visual-row index (0-based) of every requested caret offset, computed in one
// mirror build. Returns null when layout is unavailable (no real line height).
function measureRows(ta: HTMLTextAreaElement, offsets: number[]): number[] | null {
  if (typeof document === "undefined") return null;
  const div = buildMirror(ta);
  document.body.appendChild(div);
  try {
    div.textContent = "M";
    const lh = div.offsetHeight;
    if (!lh) return null; // no layout (tests) → caller uses its fallback
    return offsets.map((off) => {
      div.textContent = "";
      div.appendChild(document.createTextNode(ta.value.slice(0, off)));
      const span = document.createElement("span");
      // The char at the caret sits exactly on the caret's row; a zero-width
      // marker at end-of-text gives the final row without forcing a wrap.
      span.textContent = ta.value.slice(off, off + 1) || "​";
      div.appendChild(span);
      div.appendChild(document.createTextNode(ta.value.slice(off + 1)));
      return Math.round(span.offsetTop / lh);
    });
  } finally {
    document.body.removeChild(div);
  }
}

/** Resolve `col` characters into the textarea's LAST visual row. Returns null
 * when layout is unavailable so callers can retain their source-line fallback.
 *
 * Cross-block ArrowUp needs this distinction: a target with one long wrapped
 * source line has a bottom visual row even though `lastIndexOf("\n")` is -1. */
export function caretOffsetOnLastRow(ta: HTMLTextAreaElement, col: number): number | null {
  const end = ta.value.length;
  const endRows = measureRows(ta, [end]);
  if (!endRows) return null;
  const lastRow = endRows[0];

  // Visual row indices are monotone with caret offsets. Find the first offset
  // belonging to the final row without measuring every character in a large
  // block (cross-block navigation is rare, but should still stay logarithmic).
  let lo = 0;
  let hi = end;
  while (lo < hi) {
    const mid = Math.floor((lo + hi) / 2);
    const rows = measureRows(ta, [mid]);
    if (!rows) return null;
    if (rows[0] < lastRow) lo = mid + 1;
    else hi = mid;
  }

  return lo + Math.min(Math.max(0, col), end - lo);
}

/** Column of `offset` within its CURRENT visual row. Returns null when layout
 * is unavailable so callers can fall back to the current source-line column.
 *
 * Cross-block ArrowDown needs this distinction: on the bottom row of a long
 * wrapped source line, the source-line column includes all preceding visual
 * rows and therefore commonly clamps to the end of the next block. */
export function caretColumnOnVisualRow(ta: HTMLTextAreaElement, offset: number): number | null {
  const clamped = Math.min(Math.max(0, offset), ta.value.length);
  const rows = measureRows(ta, [clamped]);
  if (!rows) return null;
  const targetRow = rows[0];

  // Find the first caret offset on the target row. Row indices are monotone,
  // so this remains logarithmic even for very large blocks.
  let lo = 0;
  let hi = clamped;
  while (lo < hi) {
    const mid = Math.floor((lo + hi) / 2);
    const measured = measureRows(ta, [mid]);
    if (!measured) return null;
    if (measured[0] < targetRow) lo = mid + 1;
    else hi = mid;
  }

  return clamped - lo;
}

/** True if the caret at `offset` is on the FIRST visual row of the textarea (so
 *  Up / shift-select-up should leave the block). Degrades to true if unmeasurable. */
export function caretAtFirstRow(ta: HTMLTextAreaElement, offset: number): boolean {
  // The start of the value is necessarily on its first visual row. Besides
  // avoiding needless layout work, this keeps an exact boundary independent of
  // browser-specific mirror rounding.
  if (offset === 0) return true;
  const rows = measureRows(ta, [offset]);
  return rows ? rows[0] === 0 : true;
}

/** True if the caret at `offset` is on the LAST visual row of the textarea (so
 *  Down / shift-select-down should leave the block). Degrades to true if unmeasurable. */
export function caretAtLastRow(ta: HTMLTextAreaElement, offset: number): boolean {
  // The end of the value is necessarily on its last visual row. WebKitGTK can
  // round the off-screen mirror's end marker onto a phantom following row,
  // which must not trap ArrowDown at the end of an editor.
  if (offset === ta.value.length) return true;
  const rows = measureRows(ta, [offset, ta.value.length]);
  return rows ? rows[0] === rows[1] : true;
}
