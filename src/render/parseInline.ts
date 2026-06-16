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
  | { t: "image"; alt: string; url: string; width?: string; height?: string };

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

const TAG_RE = /^#([\w/_-]+)/;

function findClose(s: string, from: number, close: string): number {
  return s.indexOf(close, from);
}

export function parseInline(input: string): Seg[] {
  const out: Seg[] = [];
  let i = 0;
  let plain = "";
  const flush = () => {
    if (plain) {
      out.push({ t: "text", v: plain });
      plain = "";
    }
  };

  while (i < input.length) {
    const rest = input.slice(i);

    let m = /^!\[([^\]]*)\]\(([^)]+)\)(\{[^}]*\})?/.exec(rest);
    if (m) {
      flush();
      out.push({ t: "image", alt: m[1], url: m[2], ...parseImageMeta(m[3]) });
      i += m[0].length;
      continue;
    }
    m = /^\[([^\]]*)\]\(([^)]+)\)/.exec(rest);
    if (m) {
      flush();
      out.push({ t: "link", label: m[1], url: m[2] });
      i += m[0].length;
      continue;
    }
    // Angle autolink <https://…> / <mailto:…>
    m = /^<((?:https?:\/\/|mailto:)[^>\s]+)>/.exec(rest);
    if (m) {
      flush();
      out.push({ t: "link", label: m[1], url: m[1] });
      i += m[0].length;
      continue;
    }
    // Bare URL autolink (trailing sentence punctuation excluded)
    m = /^https?:\/\/[^\s<]+/.exec(rest);
    if (m) {
      const url = m[0].replace(/[.,;:!?)\]]+$/, "");
      flush();
      out.push({ t: "link", label: url, url });
      i += url.length;
      continue;
    }
    if (rest.startsWith("[[")) {
      const end = findClose(input, i + 2, "]]");
      if (end !== -1) {
        flush();
        out.push({ t: "pageref", name: input.slice(i + 2, end) });
        i = end + 2;
        continue;
      }
    }
    if (rest.startsWith("((")) {
      const end = findClose(input, i + 2, "))");
      if (end !== -1) {
        flush();
        out.push({ t: "blockref", id: input.slice(i + 2, end) });
        i = end + 2;
        continue;
      }
    }
    if (rest.startsWith("{{")) {
      const end = findClose(input, i + 2, "}}");
      if (end !== -1) {
        flush();
        out.push({ t: "macro", body: input.slice(i + 2, end).trim() });
        i = end + 2;
        continue;
      }
    }
    if (rest[0] === "#") {
      if (rest.startsWith("#[[")) {
        const end = findClose(input, i + 3, "]]");
        if (end !== -1) {
          flush();
          out.push({ t: "tag", name: input.slice(i + 3, end) });
          i = end + 2;
          continue;
        }
      }
      const tm = TAG_RE.exec(rest);
      if (tm) {
        flush();
        out.push({ t: "tag", name: tm[1] });
        i += tm[0].length;
        continue;
      }
    }
    if (rest[0] === "$") {
      const dbl = rest.startsWith("$$");
      const delim = dbl ? "$$" : "$";
      const end = findClose(input, i + delim.length, delim);
      if (end !== -1 && end > i + delim.length) {
        flush();
        out.push({ t: "math", tex: input.slice(i + delim.length, end), display: dbl });
        i = end + delim.length;
        continue;
      }
    }
    if (rest[0] === "`") {
      const end = findClose(input, i + 1, "`");
      if (end !== -1) {
        flush();
        out.push({ t: "code", v: input.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    const pair = matchPair(rest);
    if (pair) {
      flush();
      out.push({ t: pair.kind, v: parseInline(pair.inner) } as Seg);
      i += pair.len;
      continue;
    }

    plain += input[i];
    i++;
  }
  flush();
  return out;
}

function matchPair(
  rest: string
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
    if (rest.startsWith(d)) {
      const end = rest.indexOf(d, d.length);
      if (end !== -1 && end >= d.length) {
        const inner = rest.slice(d.length, end);
        if (inner.length > 0) return { kind, inner, len: end + d.length };
      }
    }
  }
  return null;
}
