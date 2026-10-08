import { codeFences, type LiteralContainer } from "./fences";

// GH #357: is the WHOLE visible block text one fenced code block?
// The block editor uses the answer only to stabilize its own visual box
// (same card + font metrics as the rendered code region); the buffer itself
// stays the honest raw text, fences included.

export interface CodeFenceShape {
  /** Info-string language id ("" when none). "calc" is excluded — those
   *  blocks have their own specialized editing mode. */
  lang: string;
}

/** The code container that IS the whole block's wrapper: the parser's first fence, source or
 *  example container (or the open fence being typed) starting at column zero of the text.
 *  `calc` blocks have their own editing mode and are never code wrappers. */
function wrapperOf(text: string, format: "md" | "org"): LiteralContainer | null {
  const fence = codeFences(text, format)[0];
  return fence && fence.start === 0 && fence.lang !== "calc" ? fence : null;
}

/** Whole-block fenced-code shape, matching what the renderer puts into a
 *  single code card. Mixed content (a paragraph before/after the fence,
 *  another fence after a closed fence) is NOT code-shaped and returns null,
 *  as is anything whose info string is `calc`. A fence still being typed (no
 *  closer yet) counts as code to the end of the text. Fence syntax is lsdoc's
 *  (src/editor/fences.ts), not CommonMark's. */
export function codeFenceOnly(text: string, format: "md" | "org"): CodeFenceShape | null {
  const fence = wrapperOf(text, format);
  if (!fence || (fence.closed && text.slice(fence.end).trim() !== "")) return null;
  return { lang: fence.kind === "example" ? "" : fence.lang };
}

// ---------------------------------------------------------------------------
// GH #412/#413: the body-only projection behind the code editor.
//
// One complete code body, chosen by a raw caret for mixed blocks or by the
// whole-block wrapper otherwise. open/body/close hold exact prefix/payload/suffix
// bytes. Parser ranges own the boundaries; calc and incomplete wrappers stay raw.

export interface CodeBodyProjection {
  /** Exact prefix through the opening line, INCLUDING the trailing newline. */
  open: string;
  /** Body bytes between the wrapper lines, verbatim (may be "" or end in "\n"). */
  body: string;
  /** Exact suffix from the structural separator through the end of the text. */
  close: string;
  /** Info-string language id ("" when none). */
  lang: string;
}

/** Split a complete code wrapper into exact prefix/body/suffix bytes. Without
 *  a caret, require a whole-block wrapper; with a raw UTF-16 caret, select its
 *  code body in mixed content. Null for incomplete, malformed, calc or non-code.
 *  O(block text), using the cached parser regions; never recognizes syntax. */
export function codeBodyProjection(text: string, format: "md" | "org", caret?: number): CodeBodyProjection | null {
  const fence = caret === undefined ? wrapperOf(text, format) : codeFences(text, format).find(f =>
    f.lang !== "calc" && f.openEnd <= caret && (caret < f.closeStart || f.openEnd === f.closeStart && caret === f.closeStart));
  // Incomplete wrappers (no closer yet) are still being authored; content after the
  // closer other than blank lines is mixed.
  if (!fence || !fence.closed || (caret === undefined && text.slice(fence.end).trim() !== "")) return null;
  const open = text.slice(0, fence.openEnd);
  // The final newline before the closer is wrapper structure, not editable
  // payload. Keeping it in `body` made every one-character live commit project
  // a new trailing newline back into the controlled textarea, so the next
  // character landed on a fresh line. Preserve that exact byte in `close`
  // instead; explicit payload newlines remain in `body`.
  const separatorSize = text.slice(fence.closeStart - 2, fence.closeStart) === "\r\n" ? 2 : 1;
  const structuralSeparator = Math.max(open.length, fence.closeStart - separatorSize);
  return {
    open,
    body: text.slice(open.length, structuralSeparator),
    close: text.slice(structuralSeparator),
    lang: fence.kind === "example" ? "" : fence.lang,
  };
}

/** Rebuild the raw wrapper text for an edited body. The mandatory separator
 *  before the closer belongs to `close`, so ordinary per-character commits do
 *  not leak it into the controlled textarea. Explicit body newlines remain
 *  payload. The wrapper bytes are re-attached exactly, never canonicalized. */
export function codeBodyJoin(proj: Pick<CodeBodyProjection, "open" | "close">, body: string): string {
  const separator = body !== "" && !proj.close.startsWith("\n") && !proj.close.startsWith("\r\n") ? (proj.open.endsWith("\r\n") ? "\r\n" : "\n") : "";
  return proj.open + body + separator + proj.close;
}

/** The body-space counterpart of the special-block double-Enter exit: with
 *  the caret on a TRAILING blank body line, drop that sentinel line (the
 *  caller then commits the trimmed body and creates a sibling block). Blank
 *  lines in the middle are ordinary content; an all-blank body never exits. */
export function codeBodyExitTrim(text: string, caret: number): string | null {
  const c = Math.max(0, Math.min(caret, text.length));
  const lineStart = text.lastIndexOf("\n", c - 1) + 1;
  let lineEnd = text.indexOf("\n", c);
  if (lineEnd === -1) lineEnd = text.length;
  if (text.slice(lineStart, lineEnd).trim() !== "" || lineStart === 0) return null;
  if (text.slice(lineEnd).trim() !== "") return null;
  const trimmed = text.slice(0, lineStart - 1);
  return trimmed.trim() === "" ? null : trimmed;
}
