// Pure inline-markdown parser: string -> segment list. No DOM/JSX, so it is
// unit-testable headlessly. The renderer (inline.tsx) turns segments into DOM.

export type Format = "md" | "org";

export type Seg =
  | { t: "text"; v: string }
  | { t: "bold"; v: Seg[] }
  | { t: "italic"; v: Seg[] }
  | { t: "underline"; v: Seg[] }
  | { t: "strike"; v: Seg[] }
  | { t: "highlight"; v: Seg[] }
  | { t: "code"; v: string }
  | { t: "pageref"; name: string; alias?: string }
  | { t: "tag"; name: string }
  | { t: "blockref"; id: string }
  | { t: "macro"; body: string }
  | { t: "math"; tex: string; display: boolean }
  | { t: "link"; label: string; url: string }
  | { t: "image"; alt: string; url: string; width?: string; height?: string }
  | { t: "footnote"; id: string }
  | { t: "timestamp"; raw: string; active: boolean }
  | { t: "iframe"; src: string; width?: string; height?: string };

/** Parse a Logseq image-metadata brace like `{:width 200, :height 100}`. */
function parseImageMeta(brace: string | undefined): { width?: string; height?: string } {
  if (!brace) return {};
  const out: { width?: string; height?: string } = {};
  const w = /:width\s+([0-9]+%?|[0-9]+px)/.exec(brace);
  const h = /:height\s+([0-9]+%?|[0-9]+px)/.exec(brace);
  if (w) out.width = /^\d+$/.test(w[1]) ? `${w[1]}px` : w[1];
  if (h) out.height = /^\d+$/.test(h[1]) ? `${h[1]}px` : h[1];
  return out;
}

// Sticky (`y`) regexes match at exactly `re.lastIndex`, so we scan the full
// input by index instead of allocating `input.slice(i)` every iteration (the old
// approach was O(n²) — a fresh suffix string plus a per-character `plain +=`).
const RE_IMAGE = /!\[([^\]]*)\]\(([^)]+)\)(\{[^}]*\})?/y;
const RE_FOOTNOTE = /\[\^([^\]]+)\]/y;
const RE_LINK = /\[([^\]]*)\]\(([^)]+)\)/y;
const RE_IFRAME = /<iframe\b([^>]*)>(?:\s*<\/iframe>)?/iy;
const RE_ANGLE = /<((?:https?:\/\/|mailto:)[^>\s]+)>/y;
const RE_BAREURL = /https?:\/\/[^\s<]+/y;
const RE_TAG = /#([\w/_-]+)/y;

function stickyExec(re: RegExp, input: string, i: number): RegExpExecArray | null {
  re.lastIndex = i;
  return re.exec(input);
}

export function parseInline(input: string, format: Format = "md"): Seg[] {
  const org = format === "org";
  const out: Seg[] = [];
  const len = input.length;
  let i = 0;
  // Plain text accumulates as a [plainStart, i) slice of `input` — flushed (one
  // substring, not char-by-char concat) right before each non-text segment.
  let plainStart = 0;
  const flush = (end: number) => {
    if (end > plainStart) out.push({ t: "text", v: input.slice(plainStart, end) });
  };

  while (i < len) {
    // Markdown-only inline forms (`![alt](url)`, `[^fn]`, `[label](url)`). Org
    // uses `[[..]]` / `[[target][desc]]` links instead (handled below), so these
    // delimiters stay literal in org text.
    let m: RegExpExecArray | null = null;
    if (!org) {
      m = stickyExec(RE_IMAGE, input, i);
      if (m) {
        flush(i);
        out.push({ t: "image", alt: m[1], url: m[2], ...parseImageMeta(m[3]) });
        i += m[0].length;
        plainStart = i;
        continue;
      }
      // Footnote reference `[^id]` (before the link rule; `[^id]` has no `(url)`).
      m = stickyExec(RE_FOOTNOTE, input, i);
      if (m) {
        flush(i);
        out.push({ t: "footnote", id: m[1] });
        i += m[0].length;
        plainStart = i;
        continue;
      }
      m = stickyExec(RE_LINK, input, i);
      if (m) {
        flush(i);
        out.push({ t: "link", label: m[1], url: m[2] });
        i += m[0].length;
        plainStart = i;
        continue;
      }
    }
    // Raw <iframe> embed (the safe subset of raw HTML): only an http(s) src is
    // honoured, rendered sandboxed. General raw HTML is not parsed.
    m = stickyExec(RE_IFRAME, input, i);
    if (m) {
      const attrs = m[1];
      const src = /src\s*=\s*["']([^"']+)["']/i.exec(attrs)?.[1];
      if (src && /^https?:\/\//i.test(src)) {
        const width = /width\s*=\s*["']?(\d+%?|\d+px)["']?/i.exec(attrs)?.[1];
        const height = /height\s*=\s*["']?(\d+%?|\d+px)["']?/i.exec(attrs)?.[1];
        flush(i);
        out.push({ t: "iframe", src, width, height });
        i += m[0].length;
        plainStart = i;
        continue;
      }
    }
    // Angle autolink <https://…> / <mailto:…>
    m = stickyExec(RE_ANGLE, input, i);
    if (m) {
      flush(i);
      out.push({ t: "link", label: m[1], url: m[1] });
      i += m[0].length;
      plainStart = i;
      continue;
    }
    // Bare URL autolink (trailing sentence punctuation + closing quotes/brackets
    // excluded — e.g. `"… https://x"` must not swallow the closing quote).
    m = stickyExec(RE_BAREURL, input, i);
    if (m) {
      const url = m[0].replace(/[.,;:!?)\]"'}>]+$/, "");
      flush(i);
      out.push({ t: "link", label: url, url });
      i += url.length;
      plainStart = i;
      continue;
    }
    const c = input[i];
    // Org timestamp: active `<YYYY-MM-DD …>` or inactive `[YYYY-MM-DD …]`. The
    // leading-date check keeps `[[Page]]`, `[note]`, `<https://…>` (already
    // consumed above) from matching.
    if (org && (c === "<" || c === "[")) {
      const close = c === "<" ? ">" : "]";
      const end = input.indexOf(close, i + 1);
      if (end !== -1) {
        const inner = input.slice(i + 1, end);
        if (/^\d{4}-\d{2}-\d{2}/.test(inner)) {
          flush(i);
          out.push({ t: "timestamp", raw: inner, active: c === "<" });
          i = end + 1;
          plainStart = i;
          continue;
        }
      }
    }
    if (c === "[" && input.startsWith("[[", i)) {
      const end = input.indexOf("]]", i + 2);
      if (end !== -1) {
        flush(i);
        const inner = input.slice(i + 2, end);
        if (org) {
          // Org link: `[[target]]` or `[[target][description]]`.
          const sep = inner.indexOf("][");
          const target = sep === -1 ? inner : inner.slice(0, sep);
          const desc = sep === -1 ? undefined : inner.slice(sep + 2);
          if (/^(https?:\/\/|mailto:)/i.test(target)) {
            out.push({ t: "link", label: desc || target, url: target });
          } else {
            out.push({ t: "pageref", name: orgLinkName(target), alias: desc });
          }
        } else {
          out.push({ t: "pageref", name: inner });
        }
        i = end + 2;
        plainStart = i;
        continue;
      }
    }
    if (c === "(" && input.startsWith("((", i)) {
      const end = input.indexOf("))", i + 2);
      if (end !== -1) {
        flush(i);
        out.push({ t: "blockref", id: input.slice(i + 2, end) });
        i = end + 2;
        plainStart = i;
        continue;
      }
    }
    if (c === "{" && input.startsWith("{{", i)) {
      const end = input.indexOf("}}", i + 2);
      if (end !== -1) {
        flush(i);
        out.push({ t: "macro", body: input.slice(i + 2, end).trim() });
        i = end + 2;
        plainStart = i;
        continue;
      }
    }
    if (c === "#") {
      if (input.startsWith("#[[", i)) {
        const end = input.indexOf("]]", i + 3);
        if (end !== -1) {
          flush(i);
          out.push({ t: "tag", name: input.slice(i + 3, end) });
          i = end + 2;
          plainStart = i;
          continue;
        }
      }
      const tm = stickyExec(RE_TAG, input, i);
      if (tm) {
        flush(i);
        out.push({ t: "tag", name: tm[1] });
        i += tm[0].length;
        plainStart = i;
        continue;
      }
    }
    if (c === "$") {
      const dbl = input.startsWith("$$", i);
      const delim = dbl ? "$$" : "$";
      const end = input.indexOf(delim, i + delim.length);
      if (end !== -1 && end > i + delim.length) {
        flush(i);
        out.push({ t: "math", tex: input.slice(i + delim.length, end), display: dbl });
        i = end + delim.length;
        plainStart = i;
        continue;
      }
    }
    if (!org && c === "`") {
      const end = input.indexOf("`", i + 1);
      if (end !== -1) {
        flush(i);
        out.push({ t: "code", v: input.slice(i + 1, end) });
        i = end + 1;
        plainStart = i;
        continue;
      }
    }
    if (org) {
      const e = matchOrgEmphasis(input, i);
      if (e) {
        flush(i);
        // `~code~` / `=verbatim=` are literal (no nested parse); the rest recurse.
        if (e.kind === "code") out.push({ t: "code", v: e.inner });
        else out.push({ t: e.kind, v: parseInline(e.inner, "org") } as Seg);
        i += e.len;
        plainStart = i;
        continue;
      }
    } else {
      const pair = matchPair(input, i);
      if (pair) {
        flush(i);
        out.push({ t: pair.kind, v: parseInline(pair.inner) } as Seg);
        i += pair.len;
        plainStart = i;
        continue;
      }
    }

    i++;
  }
  flush(len);
  return out;
}

function matchPair(
  input: string,
  i: number
): { kind: "bold" | "italic" | "strike" | "highlight"; inner: string; len: number } | null {
  const delims: [string, "bold" | "italic" | "strike" | "highlight"][] = [
    ["**", "bold"],
    ["__", "bold"],
    ["~~", "strike"],
    ["==", "highlight"],
    ["*", "italic"],
    ["_", "italic"],
  ];
  for (const [d, kind] of delims) {
    if (input.startsWith(d, i)) {
      const end = input.indexOf(d, i + d.length);
      if (end !== -1 && end >= i + d.length) {
        const inner = input.slice(i + d.length, end);
        if (inner.length > 0) return { kind, inner, len: end + d.length - i };
      }
    }
  }
  return null;
}

// --- org-mode inline ---------------------------------------------------------

/** Page name for an org link target. `[[Page Name]]` → the name; a `file:` /
 *  path target → its basename without extension (best-effort — Logseq itself
 *  writes plain `[[Page Name]]` for page links). */
function orgLinkName(target: string): string {
  const t = target.replace(/^file:/i, "");
  if (t.includes("/")) {
    const base = t.split("/").pop() || t;
    return base.replace(/\.(org|md)$/i, "");
  }
  return t;
}

// Org emphasis markers → segment kind. `~` (code) and `=` (verbatim) render as
// inline code (literal, no nested parse); the rest nest.
const ORG_EMPH: Record<string, "bold" | "italic" | "underline" | "strike" | "code"> = {
  "*": "bold",
  "/": "italic",
  "_": "underline",
  "+": "strike",
  "~": "code",
  "=": "code",
};
// Org emphasis boundary classes (the defaults of `org-emphasis-regexp-components`):
// the opening marker must follow one of these (or start-of-line), and the closing
// marker must precede one of these (or end-of-line). This stops plain text like
// `a/b`, `2*3`, `snake_case` or `~/path` from rendering as emphasis.
const ORG_PRE = /[\s\-('"{]/;
const ORG_POST = /[\s\-.,:!?;'")}[]/;

function matchOrgEmphasis(
  input: string,
  i: number
): { kind: "bold" | "italic" | "underline" | "strike" | "highlight" | "code"; inner: string; len: number } | null {
  // Logseq highlight `^^text^^` (a Logseq org extension, not standard org).
  if (input.startsWith("^^", i) && (i === 0 || ORG_PRE.test(input[i - 1]))) {
    const end = input.indexOf("^^", i + 2);
    if (
      end > i + 2 &&
      !/\s/.test(input[i + 2]) &&
      !/\s/.test(input[end - 1]) &&
      (input[end + 2] === undefined || ORG_POST.test(input[end + 2]))
    ) {
      return { kind: "highlight", inner: input.slice(i + 2, end), len: end + 2 - i };
    }
  }
  const ch = input[i];
  const kind = ORG_EMPH[ch];
  if (!kind) return null;
  if (i > 0 && !ORG_PRE.test(input[i - 1])) return null; // marker not at a left boundary
  const after = input[i + 1];
  if (after === undefined || /\s/.test(after)) return null; // body must start non-space
  let j = i + 1;
  for (;;) {
    const end = input.indexOf(ch, j);
    if (end === -1) return null;
    if (
      end > i + 1 &&
      !/\s/.test(input[end - 1]) && // body must end non-space
      (input[end + 1] === undefined || ORG_POST.test(input[end + 1]))
    ) {
      return { kind, inner: input.slice(i + 1, end), len: end + 1 - i };
    }
    j = end + 1;
  }
}
