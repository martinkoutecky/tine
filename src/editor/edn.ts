// Tiny, dependency-free helpers for the bits of EDN we read/write inside a
// `{{query … {:opts}}}` macro. String- and brace-aware so values containing `"`,
// `\`, `{`, or `}` don't confuse the (otherwise regex-based) query handling.

/** Escape a string for an EDN double-quoted literal. */
export function quoteEdnString(s: string): string {
  return s.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}
/** Inverse of quoteEdnString for the captured inner text of an EDN string. */
export function unquoteEdnString(inner: string): string {
  return inner.replace(/\\(.)/g, "$1");
}

// Index of the closing quote of an EDN string whose opening quote is at `i`
// (skips `\"`/`\\`); end-of-string index if unterminated.
function strClose(s: string, i: number): number {
  let j = i + 1;
  while (j < s.length) {
    const c = s[j];
    if (c === "\\") j += 2;
    else if (c === '"') return j;
    else j++;
  }
  return s.length - 1;
}

// If a Logseq page ref `[[…]]` opens at `i`, return the index just PAST its
// closing `]]` (or end-of-string if unterminated); else -1. Page refs don't
// nest, so the first `]]` closes it. Used to treat a ref's text — which may
// contain stray `{`/`}` (e.g. `[[a}}b]]`) — as opaque while scanning braces.
function pageRefEnd(s: string, i: number): number {
  if (s[i] !== "[" || s[i + 1] !== "[") return -1;
  const close = s.indexOf("]]", i + 2);
  return close === -1 ? s.length : close + 2;
}

/** Extent [start, end) of the first `{{query …}}` macro in `raw`, brace/string/
 *  page-ref-aware: a `}}` inside a string, a nested `{…}` map, or a `[[page]]`
 *  ref won't end it early. Null if there's no `{{query` macro or it's
 *  unterminated. Use this to REWRITE the macro in place — a lazy
 *  `/\{\{query…\}\}/` regex truncates at the first `}}`. */
export function queryMacroExtent(raw: string): { start: number; end: number } | null {
  const m = /\{\{query\b/i.exec(raw);
  if (!m) return null;
  const start = m.index;
  let depth = 0;
  let i = start;
  while (i < raw.length) {
    const c = raw[i];
    if (c === '"') {
      i = strClose(raw, i) + 1;
      continue;
    }
    if (c === "[") {
      const pe = pageRefEnd(raw, i);
      if (pe !== -1) {
        i = pe;
        continue;
      }
    }
    if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0) return { start, end: i + 1 };
    }
    i++;
  }
  return null; // unterminated
}

/** Extents of ALL `{{query …}}` macros in `raw`, in source order. A block can
 *  hold more than one query; a rewrite must target the RIGHT one (matching by
 *  content), not always the first. */
export function queryMacroExtents(raw: string): { start: number; end: number }[] {
  const out: { start: number; end: number }[] = [];
  let from = 0;
  while (from < raw.length) {
    const ext = queryMacroExtent(raw.slice(from));
    if (!ext) break;
    out.push({ start: from + ext.start, end: from + ext.end });
    from += ext.end;
  }
  return out;
}

/** Split a query argument into its form and a trailing balanced `{…}` options
 *  map. Brace-aware: braces inside strings (e.g. a `:title "a {b}"`) don't break
 *  it. `opts` includes the braces; both parts are trimmed. No trailing map → "". */
export function splitTrailingMap(arg: string): { form: string; opts: string } {
  const s = arg.replace(/\s+$/, "");
  if (!s.endsWith("}")) return { form: arg.trim(), opts: "" };
  let depth = 0;
  let start = -1;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === '"') {
      i = strClose(s, i);
      continue;
    }
    if (c === "[") {
      const pe = pageRefEnd(s, i);
      if (pe !== -1) {
        i = pe - 1; // -1: the loop's i++ advances onto the char past the ref
        continue;
      }
    }
    if (c === ";") {
      while (i < s.length && s[i] !== "\n") i++;
      continue;
    }
    if (c === "{") {
      if (depth === 0) start = i;
      depth++;
    } else if (c === "}") {
      depth--;
      if (depth === 0 && i === s.length - 1 && start >= 0) {
        return { form: s.slice(0, start).trim(), opts: s.slice(start).trim() };
      }
    }
  }
  return { form: arg.trim(), opts: "" };
}
