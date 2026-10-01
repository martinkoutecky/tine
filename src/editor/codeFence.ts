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
// While the whole visible block is ONE COMPLETE code wrapper, the block editor
// shows only the payload between the wrapper lines and commits re-attach the
// exact wrapper bytes. The projection is the pure contract: it splits the raw
// text into `open` (opening fence line, newline included), `body` (everything
// between the wrapper lines, verbatim) and `close` (closer line through the
// end of the block), so `open + body + close === text` always holds. Unlike
// `codeFenceOnly` (a presentation-shape detector that tolerates an unclosed
// fence while typing), the projection requires a CLOSED wrapper as lsdoc
// closes it (OG/mldoc, not CommonMark: the closer's length and character are
// not compared). Mixed content, incomplete/malformed wrappers and ```calc
// (its own editor mode) return null and keep raw editing.

export interface CodeBodyProjection {
  /** Opening fence/`#+begin` line bytes INCLUDING the trailing newline. */
  open: string;
  /** Body bytes between the wrapper lines, verbatim (may be "" or end in "\n"). */
  body: string;
  /** Closing fence/`#+end` line through the end of the text (trailing blank
   *  lines after the closer included, exactly as authored). */
  close: string;
  /** Info-string language id ("" when none). */
  lang: string;
}

/** Split a COMPLETE whole-block code wrapper into exact open/body/close
 *  bytes, or null for mixed, incomplete, malformed, calc, or non-code text. */
export function codeBodyProjection(text: string, format: "md" | "org"): CodeBodyProjection | null {
  const fence = wrapperOf(text, format);
  // Incomplete wrappers (no closer yet) are still being authored; content after the
  // closer other than blank lines is mixed.
  if (!fence || !fence.closed || text.slice(fence.end).trim() !== "") return null;
  const open = text.slice(0, fence.openEnd);
  // The final newline before the closer is wrapper structure, not editable
  // payload. Keeping it in `body` made every one-character live commit project
  // a new trailing newline back into the controlled textarea, so the next
  // character landed on a fresh line. Preserve that exact byte in `close`
  // instead; explicit payload newlines remain in `body`.
  const structuralSeparator = Math.max(open.length, fence.closeStart - 1);
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
