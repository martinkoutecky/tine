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
