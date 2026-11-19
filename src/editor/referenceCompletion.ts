import type { Format, Inline } from "../render/ast";
import { parserReady } from "../render/parse";
import { utf16ToUtf8ByteOffset, utf8ByteToUtf16Offset } from "../render/spans";
import { parse_inline_json } from "../render/wasm/lsdoc_wasm.js";
import { isBareTagPrefix } from "../tags";
import type { Trigger } from "./autocomplete";
import { offsetInLiteral } from "./inlineLiteral";

function parseInline(source: string, format: Format): Inline[] {
  return JSON.parse(parse_inline_json(source, format === "org")) ?? [];
}

function referenceKind(node: Inline): "page" | "block" | undefined {
  if (node.k === "nested_link" || (node.k === "link" && node.url.type === "page_ref")) return "page";
  if (node.k === "link" && node.url.type === "block_ref") return "block";
}

function inlineChildren(node: Inline): Inline[] | undefined {
  if ("children" in node) return node.children;
  if (node.k === "link") return node.label;
}

function closedAt(nodes: Inline[], marker: number, afterBackslash = false): Inline | "literal" | undefined {
  for (const node of nodes) {
    if (!node.span || marker < node.span[0] || marker >= node.span[1]) continue;
    if (referenceKind(node) || node.k === "macro") return node;
    if (node.k === "code" || node.k === "verbatim" || node.k === "latex") return "literal";
    if (node.k === "link" && node.url.type === "complex") {
      // A bare or angle URL's synthetic label covers its entire URL source.
      const label = node.label?.[0];
      if (label?.k === "plain" && label.span && (
        (label.span[0] === node.span[0] && label.span[1] === node.span[1]) ||
        (label.span[0] === node.span[0] + 1 && label.span[1] === node.span[1] - 1)
      )) return "literal";
    }
    if (afterBackslash && node.k === "plain" && node.span_map) {
      let previousEnd = node.span[0];
      for (const [, start, length] of node.span_map) {
        if (start === marker && previousEnd < marker) return "literal";
        previousEnd = start + length;
      }
    }
    const children = inlineChildren(node);
    if (children) return closedAt(children, marker, afterBackslash);
  }
}

function linePrefix(chunk: string): string {
  const lf = chunk.indexOf("\n"), cr = chunk.indexOf("\r");
  return chunk.slice(0, Math.min(lf === -1 ? chunk.length : lf, cr === -1 ? chunk.length : cr));
}

function literalAhead(raw: string, lineStart: number, caret: number, marker: number, format: Format): boolean {
  const closers = format === "md" ? /[`$>]|\\[)\]]/ : /[~=$>]|\\[)\]]/;
  let scanned = caret;
  for (let size = Math.max(32, caret - lineStart); scanned < raw.length; size *= 2) {
    const limit = Math.min(caret + size, raw.length);
    const from = raw[scanned - 1] === "\\" ? scanned - 1 : scanned;
    const chunk = linePrefix(raw.slice(from, limit));
    const end = from + chunk.length;
    if (end > scanned && closers.test(chunk) &&
        closedAt(parseInline(raw.slice(lineStart, end), format), marker) === "literal") return true;
    if (end < limit) return false;
    scanned = limit;
  }
  return false;
}

function multilineLiteralAt(raw: string, at: number, format: Format): boolean {
  const delimiters = format === "md" ? ["``", "$$"] : ["$$"];
  return delimiters.some(delimiter => {
    const open = raw.lastIndexOf(delimiter, at - 1);
    if (open === -1) return false;
    const close = raw.indexOf(delimiter, at + 1);
    return close !== -1 && raw.slice(open + 2, close).includes("\n");
  }) && offsetInLiteral(raw, format, at);
}

function overlapsCloser(nodes: Inline[], closerStart: number): boolean {
  return nodes.some(node => node.span && node.span[0] < closerStart + 2 && node.span[1] > closerStart && (
    // A bare tag's plain name can include closing punctuation. A bracketed tag has a reference child.
    node.k === "tag" ? overlapsCloser(node.children, closerStart) : node.k !== "plain"
  ));
}

/** FORK-853b: finish a reference at the caret when it has no closer of its own.
 * lsdoc can absorb a closer from later text, so check its ownership by parsing
 * that text independently. A nested page reference can still have an outer closer. */
function referenceEnd(raw: string, start: number, caret: number, kind: "page" | "block", format: Format,
  nestedOnly = false): number {
  const closer = kind === "page" ? "]]" : "))";
  // Empty editor auto-pairs are scaffolds. lsdoc requires a nonempty name.
  if (!nestedOnly && caret === start + 2 && raw.startsWith(closer, caret)) return caret + 2;
  if (!parserReady()) return caret;

  // Delimiters only schedule parser probes. Start with enough room to amortize
  // rereading the reference prefix and double until lsdoc resolves the span.
  let scanned = caret;
  let replaceEnd = caret;
  let probeLaterConstruct = false;
  for (let size = Math.max(32, caret - start); ; size *= 2) {
    const limit = Math.min(caret + size, raw.length);
    const from = Math.max(start + 2, scanned - 1);
    const chunk = linePrefix(raw.slice(from, limit));
    const end = from + chunk.length;
    const done = end < limit || limit === raw.length;
    scanned = limit;
    if (!chunk.includes(closer) && !(probeLaterConstruct && /[\])}>`$~=_*^+/\\]/.test(chunk))) {
      if (done) break;
      continue;
    }

    const sample = raw.slice(start, end);
    const owner = parseInline(sample, format)[0];
    replaceEnd = caret;
    if (owner?.span?.[0] === 0 && referenceKind(owner) === kind) {
      const candidate = start + utf8ByteToUtf16Offset(sample, owner.span[1]);
      if (candidate <= caret) return caret;
      const nested = owner.k === "nested_link";
      const beforeCloser = sample.slice(2, candidate - start - 2);
      const checkLaterConstruct = /\(\(|\{\{|[\[<`$~=_*^+\\]/.test(beforeCloser);
      // The opener and closer each occupy two bytes in lsdoc's spans.
      const borrowed = checkLaterConstruct && overlapsCloser(parseInline(sample.slice(2), format), owner.span[1] - 4);
      if (!borrowed && (!nestedOnly || nested)) replaceEnd = candidate;
      const mayNest = kind === "page" && !nested && beforeCloser.includes("[[");
      if (!checkLaterConstruct || (borrowed && !mayNest)) break;
      // Once the closer is borrowed, only another closer can complete the outer reference.
      probeLaterConstruct = !borrowed;
    }
    if (done) break;
  }
  return replaceEnd;
}

/** Owns the active reference marker, its query and its replacement span.
 * Checks the current line for inline ownership and uses block regions for
 * possible multiline literals. Right-hand delimiters bound the inline probes. */
export function referenceCompletion(raw: string, before: string, lineStart: number, caret: number, format: Format): Trigger | null {
  const page = before.lastIndexOf("[[");
  const block = before.lastIndexOf("((");
  const hash = before.lastIndexOf("#");
  const tag = hash >= 0 && (hash === 0 || /\s/u.test(before[hash - 1])) && isBareTagPrefix(before.slice(hash + 1));
  const marker = Math.max(page, block);
  if (marker === -1 && !tag) return null;

  let prefix: Inline[] | undefined;
  const closed = (at: number) => {
    if (!parserReady()) return undefined;
    if (lineStart > 0 && multilineLiteralAt(raw, lineStart + at, format)) return "literal" as const;
    const afterBackslash = before[at - 1] === "\\";
    const mayOpenLiteral = (format === "md" ? /[`$<]|\\(?:\[|\()/ : /[~=$<]|\\(?:\[|\()/).test(before.slice(0, at));
    if (!afterBackslash && !mayOpenLiteral && !before.includes("://") && !/\]\]|\)\)|\}\}/.test(before.slice(at))) return undefined;
    prefix ??= parseInline(before, format);
    const marker = utf16ToUtf8ByteOffset(before, at);
    const owner = closedAt(prefix, marker, afterBackslash);
    if (owner) return owner;
    if (mayOpenLiteral && literalAhead(raw, lineStart, caret, marker, format)) return "literal" as const;
    if (afterBackslash && format === "org") {
      // Org retains the escape byte in plain text. Ask lsdoc if the same
      // opener can make a reference with a minimal complete name.
      const sample = before.slice(0, at + 2) + (before[at] === "[" ? "x]]" : "x))");
      if (!closedAt(parseInline(sample, format), marker)) return "literal" as const;
    }
    if (!before.endsWith("))") || raw[caret] !== ")") return undefined;
    // An unfinished Markdown wrapper can parse as a search URL that borrows
    // the last ')' of a block reference. Include the wrapper's next token.
    return closedAt(parseInline(before + ")", format), marker);
  };
  if (marker >= 0) {
    const kind = page > block ? "page" : "block";
    const ended = closed(marker);
    const query = before.slice(marker + 2);
    const start = lineStart + marker;
    if (!ended) {
      const pending = !(kind === "page" ? /[\[\]]/ : /[()]/).test(query);
      const replaceEnd = referenceEnd(raw, start, caret, kind, format);
      // GH #888 (D-20261109-LK): rank the whole existing page name. Logseq only
      // uses the prefix through the caret (6e7afa8eb, components/editor.cljs:105–121).
      if (pending || replaceEnd > caret) return { kind, start, end: caret,
        query: kind === "page" && replaceEnd > caret ? raw.slice(start + 2, replaceEnd - 2) : query,
        ...(replaceEnd > caret ? { replaceEnd } : {}) };
    }
    if (!(tag && hash > marker) && ended !== "literal" && ended?.k === "link" && ended.url.type === "page_ref" && ended.span) {
      const outer = lineStart + utf8ByteToUtf16Offset(before, ended.span[0]);
      if (outer < lineStart + page && raw.startsWith("[[", outer)) {
        const replaceEnd = referenceEnd(raw, outer, caret, "page", format, true);
        if (replaceEnd > caret) return { kind: "page", start: outer, end: caret, replaceEnd,
          query: raw.slice(outer + 2, replaceEnd - 2) };
      }
    }
  }
  if (tag && !closed(hash)) return { kind: "tag", query: before.slice(hash + 1), start: lineStart + hash, end: caret };
  return null;
}
