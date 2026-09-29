// GH #357: is the WHOLE visible block text one fenced code block?
// The block editor uses the answer only to stabilize its own visual box
// (same card + font metrics as the rendered code region); the buffer itself
// stays the honest raw text, fences included.

export interface CodeFenceShape {
  /** Info-string language id ("" when none). "calc" is excluded — those
   *  blocks have their own specialized editing mode. */
  lang: string;
}

const MD_FENCE = /^(`{3,}|~{3,})\s*([^\n]*?)\s*$/;
const ORG_SRC = /^#\+begin_src\s+([^\s]+)\s*$/i;
const ORG_EXAMPLE = /^#\+begin_example\s*$/i;
const ORG_END = /^#\+end_(src|example)\s*$/i;

function langOf(info: string): string {
  return info.trim().split(/\s+/)[0]?.toLowerCase() ?? "";
}

const MD_CLOSE = /^(`{3,}|~{3,})$/;

/** Every line after the fence's CLOSER (a run of 3+ fence chars alone, or an
 *  org #+END_ line) is blank, or there is no closer at all — an unclosed fence
 *  still renders as code to end-of-block while the user is typing it. A code
 *  line that merely starts with a fence char (`\`x\``) is never a closer. */
function closesAtEnd(lines: string[], start: number, isClose: (trimmed: string) => boolean): boolean {
  const inner = lines.slice(start);
  const closeIdx = inner.findIndex((line) => isClose(line.trim()));
  if (closeIdx === -1) return true; // unclosed fence: everything so far is code
  return inner.slice(closeIdx + 1).every((line) => line.trim() === "");
}

/** Whole-block fenced-code shape, matching what the renderer puts into a
 *  single code card. Mixed content (a paragraph before/after the fence,
 *  another fence after a closed fence) is NOT code-shaped and returns null,
 *  as is anything whose info string is `calc`. */
export function codeFenceOnly(text: string, format: "md" | "org"): CodeFenceShape | null {
  const lines = text.split("\n");
  const first = lines[0] ?? "";
  if (format === "md") {
    const m = MD_FENCE.exec(first);
    if (!m) return null;
    const lang = langOf(m[2] ?? "");
    if (lang === "calc") return null;
    if (!closesAtEnd(lines, 1, (t) => MD_CLOSE.test(t))) return null;
    return { lang };
  }
  const src = ORG_SRC.exec(first);
  if (src) {
    const lang = langOf(src[1] ?? "");
    if (lang === "calc") return null;
    if (!closesAtEnd(lines, 1, (t) => ORG_END.test(t))) return null;
    return { lang };
  }
  if (ORG_EXAMPLE.test(first)) {
    if (!closesAtEnd(lines, 1, (t) => ORG_END.test(t))) return null;
    return { lang: "" };
  }
  return null;
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
// fence while typing), the projection requires a CLOSED wrapper with the
// CommonMark closing rule — the closer uses the opener's character and is at
// least as long. Mixed content, incomplete/malformed wrappers and ```calc
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

/** Find the wrapper's closer line index (≥ 1) under the CommonMark closing
 *  rule, or -1 when no line closes the wrapper. */
function closerLineIndex(lines: string[], format: "md" | "org", fence: { char: "`" | "~"; length: number } | null): number {
  for (let i = 1; i < lines.length; i++) {
    const trimmed = lines[i].trim();
    if (format === "org") {
      if (ORG_END.test(trimmed)) return i;
      continue;
    }
    const run = /^(`{3,}|~{3,})$/.exec(trimmed);
    if (run && fence && run[1][0] === fence.char && run[1].length >= fence.length) return i;
  }
  return -1;
}

/** Split a COMPLETE whole-block code wrapper into exact open/body/close
 *  bytes, or null for mixed, incomplete, malformed, calc, or non-code text. */
export function codeBodyProjection(text: string, format: "md" | "org"): CodeBodyProjection | null {
  const lines = text.split("\n");
  if (lines.length < 2) return null;
  const first = lines[0];
  let lang: string;
  let fence: { char: "`" | "~"; length: number } | null = null;
  if (format === "md") {
    const m = MD_FENCE.exec(first);
    if (!m) return null;
    lang = langOf(m[2] ?? "");
    if (lang === "calc") return null;
    fence = { char: m[1][0] as "`" | "~", length: m[1].length };
  } else {
    const src = ORG_SRC.exec(first);
    if (src) {
      lang = langOf(src[1] ?? "");
      if (lang === "calc") return null;
    } else if (ORG_EXAMPLE.test(first)) {
      lang = "";
    } else {
      return null;
    }
  }
  const closer = closerLineIndex(lines, format, fence);
  if (closer === -1) return null; // incomplete: still being authored
  // Content after the closer (other than trailing blank lines) is mixed.
  if (!lines.slice(closer + 1).every((line) => line.trim() === "")) return null;
  const open = first + "\n";
  let closeStart = open.length;
  for (let i = 1; i < closer; i++) closeStart += lines[i].length + 1;
  // The final newline before the closer is wrapper structure, not editable
  // payload. Keeping it in `body` made every one-character live commit project
  // a new trailing newline back into the controlled textarea, so the next
  // character landed on a fresh line. Preserve that exact byte in `close`
  // instead; explicit payload newlines remain in `body`.
  const structuralSeparator = Math.max(open.length, closeStart - 1);
  return {
    open,
    body: text.slice(open.length, structuralSeparator),
    close: text.slice(structuralSeparator),
    lang,
  };
}

/** Rebuild the raw wrapper text for an edited body. The mandatory separator
 *  before the closer belongs to `close`, so ordinary per-character commits do
 *  not leak it into the controlled textarea. Explicit body newlines remain
 *  payload. The wrapper bytes are re-attached exactly, never canonicalized. */
export function codeBodyJoin(proj: Pick<CodeBodyProjection, "open" | "close">, body: string): string {
  return proj.open + body + proj.close;
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
