// Pure text operations for the block editor: inline-format toggles, link
// insertion, and Emacs-style cursor/kill motions. No DOM — each function takes
// the raw text + selection and returns the new text + selection, so Block.tsx
// just applies the result to the textarea. Unit-testable.

import { headerTokens } from "../markers";
import { parseBlock, parserReady } from "../render/parse";
import { BLOCK_COLOR_NAMES, HIGHLIGHT_FALLBACK } from "../blockColors";

export interface Edit {
  text: string;
  start: number;
  end: number;
  /** Preserve which end of a live textarea selection is active. */
  direction?: "forward" | "backward" | "none";
}

export type InlineFormat = "bold" | "italic" | "strikethrough" | "highlight";

const INLINE_FORMAT_DELIMITERS: Record<"md" | "org", Record<InlineFormat, string>> = {
  md: { bold: "**", italic: "*", strikethrough: "~~", highlight: "==" },
  org: { bold: "*", italic: "/", strikethrough: "+", highlight: "^^" },
};

const ASCII_WHITESPACE = /[\t\n\v\f\r ]/;

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

/** Toggle a parser-recognized inline format. Browser word selection commonly
 * includes the adjacent space (notably Ctrl+Shift+Left on Windows). OG trims
 * that outer whitespace before adding delimiters; keep the bytes in place and
 * retain Tine's live inner selection. Generic wrappers such as page links and
 * inline code deliberately keep using toggleWrap: their whitespace semantics
 * are a separate contract. */
export function toggleInlineFormat(
  text: string,
  start: number,
  end: number,
  format: "md" | "org",
  kind: InlineFormat,
  direction?: "forward" | "backward" | "none",
): Edit {
  let innerStart = start;
  let innerEnd = end;
  if (start !== end) {
    while (innerStart < innerEnd && ASCII_WHITESPACE.test(text[innerStart])) innerStart += 1;
    while (innerEnd > innerStart && ASCII_WHITESPACE.test(text[innerEnd - 1])) innerEnd -= 1;
    if (innerStart === innerEnd) {
      const unchanged: Edit = { text, start, end };
      return direction === undefined ? unchanged : { ...unchanged, direction };
    }
  }
  const edit = toggleWrap(text, innerStart, innerEnd, INLINE_FORMAT_DELIMITERS[format][kind]);
  return direction === undefined ? edit : { ...edit, direction };
}

/** The opening tag of a coloured highlight: Logseq's theme variable for
 * `color`, one of its built-in colors, with a fixed fallback for other viewers. Logseq renders
 * it (mldoc reads `<mark …>…</mark>` as one Inline_Html; DOMPurify keeps the style), and so does
 * Tine's sanitizer, which admits exactly this declaration. */
export function highlightMarkOpen(color: string): string {
  return `<mark style="background-color: var(--ls-highlight-color-${color}, ${HIGHLIGHT_FALLBACK[color]})">`;
}
const MARK_CLOSE = "</mark>";
const PLAIN_HIGHLIGHT = INLINE_FORMAT_DELIMITERS.md.highlight;
// Text an HTML parser reads differently from Markdown: a tag opener or a character reference.
const HTML_SIGNIFICANT = /<|&(?:#\d+|#x[0-9a-f]+|[a-z][a-z0-9]*);/i;

/** The block's AST with positions dropped and adjacent plain text merged; the highlight
 * written as `markup` reads as its `inner` text, and `seen.n` counts those (an identical
 * highlight elsewhere in the block counts too). */
function shape(value: unknown, markup: string, inner: string, seen: { n: number }): unknown {
  if (Array.isArray(value)) {
    const out: unknown[] = [];
    for (const item of value) {
      const next = shape(item, markup, inner, seen) as { k?: string; text?: string };
      const last = out[out.length - 1] as { k?: string; text?: string } | undefined;
      if (next?.k === "plain" && last?.k === "plain") out[out.length - 1] = { k: "plain", text: last.text! + next.text! };
      else out.push(next);
    }
    return out;
  }
  if (!value || typeof value !== "object") return value;
  const node = value as Record<string, unknown>;
  if (node.k === "inline_html" && node.text === markup) {
    seen.n += 1;
    return { k: "plain", text: inner };
  }
  if (node.k === "plain") return { k: "plain", text: node.text };
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(node)) if (key !== "span") out[key] = shape(node[key], markup, inner, seen);
  return out;
}

/** Colour the selection's highlight (Markdown only). `color` is one of
 * Logseq's built-in colors and wraps the selection in its `<mark style=…>` (replacing a
 * coloured highlight or a plain `==` highlight already around or inside the selection);
 * `null` removes a coloured highlight. Outer whitespace stays outside, as in
 * `toggleInlineFormat`, and the selection afterwards covers the same text.
 *
 * HTML content is not Markdown: Logseq and Tine show it literally, and a link inside it
 * stops being a link. So a colour is written only around plain text that the parser reads
 * the same way after the edit: the block's AST, with the new highlight read as its text,
 * must equal the AST before. Otherwise, before the parser is ready, and when there is
 * nothing to remove, the answer is `null` and the text stays as it is. O(block bytes):
 * two parses of the block and one of the selection. */
export function setHighlightColor(
  text: string,
  start: number,
  end: number,
  color: string | null,
  direction?: "forward" | "backward" | "none",
): Edit | null {
  if (!parserReady()) return null;
  let s = start, e = end;
  while (s < e && ASCII_WHITESPACE.test(text[s])) s += 1;
  while (e > s && ASCII_WHITESPACE.test(text[e - 1])) e -= 1;
  if (s === e) return null;
  // The wrapper already around (or included in) the selection, if any.
  let base = text, wrapped: string | null = null;
  for (const open of [...BLOCK_COLOR_NAMES.map(highlightMarkOpen), PLAIN_HIGHLIGHT]) {
    const close = open === PLAIN_HIGHLIGHT ? PLAIN_HIGHLIGHT : MARK_CLOSE;
    const sel = text.slice(s, e);
    if (s >= open.length && text.slice(s - open.length, s) === open && text.slice(e, e + close.length) === close) {
      base = text.slice(0, s - open.length) + sel + text.slice(e + close.length);
      s -= open.length; e -= open.length;
    } else if (sel.length >= open.length + close.length && sel.startsWith(open) && sel.endsWith(close)) {
      base = text.slice(0, s) + sel.slice(open.length, sel.length - close.length) + text.slice(e);
      e -= open.length + close.length;
    } else continue;
    wrapped = open;
    break;
  }
  const withDirection = (edit: Edit): Edit => (direction === undefined ? edit : { ...edit, direction });
  if (color === null) {
    return wrapped && wrapped !== PLAIN_HIGHLIGHT ? withDirection({ text: base, start: s, end: e }) : null;
  }
  if (!(color in HIGHLIGHT_FALLBACK)) return null;
  const inner = base.slice(s, e);
  if (/[\r\n]/.test(inner) || HTML_SIGNIFICANT.test(inner)) return null;
  const [alone, ...more] = parseBlock(inner, false);
  const only = alone && "inline" in alone && Array.isArray(alone.inline) && alone.inline.length === 1 ? alone.inline[0] : null;
  if (more.length || only?.k !== "plain" || only.text !== inner || only.span_map) return null;
  const open = highlightMarkOpen(color);
  const markup = open + inner + MARK_CLOSE;
  const next = base.slice(0, s) + markup + base.slice(e);
  const seenAfter = { n: 0 }, seenBefore = { n: 0 };
  const after = JSON.stringify(shape(parseBlock(next, false), markup, inner, seenAfter));
  const before = JSON.stringify(shape(parseBlock(base, false), markup, inner, seenBefore));
  if (seenAfter.n !== seenBefore.n + 1 || after !== before) return null;
  return withDirection({ text: next, start: s + open.length, end: s + open.length + inner.length });
}

/** Whether `text` is exactly one complete inline link in mldoc's own reading (`mldoc-link?`): the
 * parser (I-12) parses it, and it must come back as a single non-image link node spanning the whole
 * text. Page refs, block refs and already formatted links are link nodes too; surrounding prose is
 * never silently promoted. Before the parser is ready nothing is recognized (the selection stays a
 * label), which is the safe direction. */
export function isMldocLink(text: string, format: "md" | "org" = "md"): boolean {
  if (!text || text !== text.trim() || !parserReady()) return false;
  const [block, ...rest] = parseBlock(text, format === "org");
  if (rest.length || !block || !("inline" in block) || !Array.isArray(block.inline)) return false;
  const [only, ...more] = block.inline;
  return more.length === 0 && only?.k === "link" && !only.image && only.full === text;
}

/** Insert a format-aware external link. Selected parser-recognized inline links
 * become the target with an empty label; ordinary selected text remains the
 * label. This matches OG's no-argument html-link-format! branches. */
export function insertLink(text: string, start: number, end: number, format: "md" | "org" = "md"): Edit {
  const sel = text.slice(start, end);
  const recognizedLink = !!sel && isMldocLink(sel, format);
  if (format === "org") {
    const link = recognizedLink ? `[[${sel}][]]` : sel ? `[[][${sel}]]` : "[[][]]";
    const caret = recognizedLink ? start + sel.length + 4 : start + 2;
    const next = text.slice(0, start) + link + text.slice(end);
    return { text: next, start: caret, end: caret };
  }
  if (recognizedLink) {
    const next = text.slice(0, start) + `[](${sel})` + text.slice(end);
    return { text: next, start: start + 1, end: start + 1 };
  }
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

// Exact OG video-provider patterns, transcribed from 6e7afa8eb
// src/main/frontend/util/text.cljs:12-23. Keep handler/paste.cljs:137-146's
// selected-URL branch ahead of this bare-URL normalization at the call site.
const VIDEO_PASTE_RE = [
  /^((?:https?:)?\/\/)?((?:www).)?((?:bilibili.com))(\/(?:video\/)?)([\w-]+)(\?p=(\d+))?(\S+)?$/,
  /^((?:https?:)?\/\/)?((?:www).)?((?:loom.com))(\/(?:share\/|embed\/))([\w-]+)(\S+)?$/,
  /^((?:https?:)?\/\/)?((?:www).)?((?:player.vimeo.com|vimeo.com))(\/(?:video\/)?)([\w-]+)(\S+)?$/,
  /^((?:https?:)?\/\/)?((?:www|m).)?((?:youtube.com|youtu.be|y2u.be|youtube-nocookie.com))(\/(?:[\w-]+\?v=|embed\/|v\/)?)([\w-]+)([\S^?]+)?$/,
];

export function videoPasteMacro(text: string): string | null {
  return VIDEO_PASTE_RE.some((pattern) => pattern.test(text)) ? `{{video ${text}}}` : null;
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
// The marker and priority spans are the parser's accepted header tokens (src/markers.ts
// `headerTokens`, I-12): this writer only splices at them.

/** Set (`level`), replace, or remove (`null`) the `[#X]` priority on the first line of `raw`,
 *  placed after any task marker. Mirrors OG's add-or-update-priority. Text after the first line
 *  is returned unchanged. */
export function setPriority(raw: string, level: "A" | "B" | "C" | null): string {
  const lineEnd = raw.indexOf("\n") === -1 ? raw.length : raw.indexOf("\n");
  const { marker, priority } = headerTokens(raw);
  const onLine = (t: { end: number } | null) => (t && t.end <= lineEnd ? t : null);
  const m = onLine(marker);
  const p = onLine(priority);
  const head = m ? raw.slice(0, m.end) : ""; // "TODO" (plus any lead ws)
  const rest = raw.slice(p ? p.end : (m?.end ?? 0), lineEnd).replace(/^\s+/, ""); // body after the tokens
  const prefix = head ? `${head} ` : "";
  const title = level ? (rest ? `${prefix}[#${level}] ${rest}` : `${prefix}[#${level}]`) : `${prefix}${rest}`;
  return title + raw.slice(lineEnd);
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
