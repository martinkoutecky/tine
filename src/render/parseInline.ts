// Pure inline-markdown parser: string -> segment list. No DOM/JSX, so it is
// unit-testable headlessly. The renderer (inline.tsx) turns segments into DOM.

export type Seg =
  | { t: "text"; v: string }
  | { t: "bold"; v: Seg[] }
  | { t: "italic"; v: Seg[] }
  | { t: "strike"; v: Seg[] }
  | { t: "highlight"; v: Seg[] }
  | { t: "code"; v: string }
  | { t: "pageref"; name: string }
  | { t: "tag"; name: string }
  | { t: "blockref"; id: string }
  | { t: "macro"; body: string }
  | { t: "math"; tex: string; display: boolean }
  | { t: "link"; label: string; url: string }
  | { t: "image"; alt: string; url: string; width?: string; height?: string }
  | { t: "footnote"; id: string }
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

export function parseInline(input: string): Seg[] {
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
    let m = stickyExec(RE_IMAGE, input, i);
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
    if (c === "[" && input.startsWith("[[", i)) {
      const end = input.indexOf("]]", i + 2);
      if (end !== -1) {
        flush(i);
        out.push({ t: "pageref", name: input.slice(i + 2, end) });
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
    if (c === "`") {
      const end = input.indexOf("`", i + 1);
      if (end !== -1) {
        flush(i);
        out.push({ t: "code", v: input.slice(i + 1, end) });
        i = end + 1;
        plainStart = i;
        continue;
      }
    }
    const pair = matchPair(input, i);
    if (pair) {
      flush(i);
      out.push({ t: pair.kind, v: parseInline(pair.inner) } as Seg);
      i += pair.len;
      plainStart = i;
      continue;
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
