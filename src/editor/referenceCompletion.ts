import type { Format, Inline } from "../render/ast";
import { parserReady } from "../render/parse";
import { utf16ToUtf8ByteOffset, utf8ByteToUtf16Offset } from "../render/spans";
import { parse_inline_json } from "../render/wasm/lsdoc_wasm.js";
import { isBareTagPrefix } from "../tags";
import type { Trigger } from "./autocomplete";
import { offsetInLiteral } from "./inlineLiteral";

function parseInline(source: string, format: Format): Inline[] | null {
  return JSON.parse(parse_inline_json(source, format === "org"));
}

function isEscaped(raw: string, at: number): boolean {
  let start = at;
  while (raw[start - 1] === "\\") start--;
  return (at - start) % 2 !== 0;
}

// Delimiters bound parser input. Only lsdoc can confirm a reference or a literal.
function referenceHorizons(raw: string, from: number, caret: number): [start: number, end: number, alternate: number] {
  const tokens = /\[\[|\]\]|\(\(|\)|[\r\n]/g;
  tokens.lastIndex = from;
  let pages = 0;
  let simplePage = false;
  let pageStart = Infinity, blockStart = Infinity;
  let pageEnd = caret, blockEnd = caret;
  for (let token = tokens.exec(raw); token; token = tokens.exec(raw)) {
    const at = token.index;
    if ((at >= caret && pages === 0 && !simplePage && blockStart === Infinity) || token[0] === "\r" || token[0] === "\n") break;
    if (token[0] === "[[") {
      if (at < caret) {
        if (pages === 0 && !simplePage) pageStart = at;
        simplePage = true;
      }
      if (at < caret || pages > 0) pages++;
    }
    if (token[0] === "]]") {
      // The parser tries both nested (raw bracket pairs) and simple (escaped) names.
      if (pages > 0) pageEnd = Math.max(pageEnd, at + 2);
      pages = Math.max(0, pages - 1);
      const close = at + Number(isEscaped(raw, at));
      if (simplePage && raw[close + 1] === "]") {
        pageEnd = Math.max(pageEnd, close + 2);
        simplePage = false;
      }
      if (pages === 0 && !simplePage && pageEnd === caret) pageStart = Infinity;
    }
    if (token[0] === "((" && at < caret) blockStart = Math.min(blockStart, at);
    if (token[0] === ")" && blockStart !== Infinity && blockEnd === caret) {
      if (raw[at + 1] === ")" && at + 2 > caret) blockEnd = at + 2;
      else blockStart = Infinity;
    }
    if (tokens.lastIndex >= caret && (blockStart === Infinity || blockEnd > caret) &&
        ((pages === 0 && !simplePage) || raw.indexOf("]]", tokens.lastIndex) === -1)) break;
  }
  // If both kinds have a closer, offer lsdoc the outer candidate first. The
  // other candidate is only needed when the outer one is rejected.
  return pageEnd > caret && (blockEnd === caret || pageStart < blockStart)
    ? [pageStart, pageEnd, blockEnd] : [blockStart, blockEnd, pageEnd];
}

const COMMON_LITERALS = [
  ["<", ">"], ["\\(", "\\)"], ["\\[", "\\]"], ["$", "$", true],
] as const;
const LITERAL_PAIRS = {
  md: [...COMMON_LITERALS, ["`", "`", true]],
  org: [...COMMON_LITERALS, ["~", "~"], ["=", "="]],
} as const;
const TITLE_PAIRS = {
  md: [...LITERAL_PAIRS.md, ["{{", "}}"], ["*", "*", true], ["_", "_", true], ["~~", "~~"], ["==", "=="]],
  org: [...LITERAL_PAIRS.org, ["{{", "}}"], ["*", "*"], ["/", "/"], ["_", "_"], ["+", "+"]],
} as const;

type DelimiterPair = readonly [open: string, close: string, run?: boolean];
type SkipOpener = (offset: number, token: string) => boolean;

function pairedContext(raw: string, prefix: string, from: number, at: number, pairs: readonly DelimiterPair[], skip?: SkipOpener) {
  let start = at;
  let end = at;
  for (const [open, close, run] of pairs) {
    let opener = prefix.lastIndexOf(open);
    while (opener >= 0 && skip?.(from + opener, open)) opener = opener === 0 ? -1 : prefix.lastIndexOf(open, opener - 1);
    if (opener === -1 || (open !== close && prefix.lastIndexOf(close) > opener)) continue;
    const after = Math.max(from + opener + open.length, at - close.length + 1);
    const terminator = raw.indexOf(close, after);
    if (terminator === -1 || /[\r\n]/.test(raw.slice(after, terminator))) continue;
    let right = terminator + close.length;
    if (run) while (raw[right] === close) right++;
    if (close === "}}" && raw[right] === "}") right++;
    start = Math.min(start, from + opener);
    end = Math.max(end, right);
  }
  return { start, end };
}

function markdownDestinationEnd(raw: string, prefix: string, from: number, at: number): number {
  const delimiter = prefix.lastIndexOf("](");
  if (delimiter === -1 || !prefix.slice(0, delimiter).includes("[")) return at;
  const parens = /\\[^\r\n]|[()<>\r\n]/g;
  parens.lastIndex = from + delimiter + 2;
  let depth = 1;
  let angle = false;
  for (let token = parens.exec(raw); token; token = parens.exec(raw)) {
    if (token[0] === "\r" || token[0] === "\n") break;
    if (token[0] === "<") angle = true;
    if (token[0] === ">") angle = false;
    if (angle) continue;
    if (token[0] === "(") depth++;
    if (token[0] === ")" && --depth === 0) return Math.max(at, token.index + 1);
  }
  return at;
}

// Extend a single parse to plausible terminators for constructs already begun in the prefix.
function contextHorizon(raw: string, from: number, at: number, format: Format, title = false, skip?: SkipOpener): number {
  const prefix = raw.slice(from, at);
  const destination = format === "md" ? markdownDestinationEnd(raw, prefix, from, at) : at;
  return Math.max(destination, pairedContext(raw, prefix, from, at, (title ? TITLE_PAIRS : LITERAL_PAIRS)[format], skip).end);
}

function multilineLiteralAt(raw: string, at: number, format: Format): boolean {
  const delimiters = format === "md" ? ["``", "$$"] : ["$$"];
  return delimiters.some(delimiter => {
    const open = raw.lastIndexOf(delimiter, at - 1);
    if (open === -1) return false;
    const close = raw.indexOf(delimiter, at);
    return close !== -1 && raw.slice(open + 2, close).includes("\n");
  }) && offsetInLiteral(raw, format, at);
}

type Reference = { kind: "page" | "block"; start: number; end: number };
type InlineOwner = Reference | "literal" | "macro";

function ownerAt(nodes: Inline[], source: string, at: number, byte = utf16ToUtf8ByteOffset(source, at)): InlineOwner | undefined {
  for (const node of nodes) {
    if (!node.span || byte < node.span[0] || byte >= node.span[1]) continue;
    if (node.k === "macro") return "macro";
    if (node.k === "code" || node.k === "verbatim" || node.k === "latex" || node.k === "email") return "literal";
    if (node.k === "plain" && source[at - 1] === "\\" && node.span_map) {
      let previousEnd = node.span[0];
      for (const [, start, length] of node.span_map) {
        if (start === byte && previousEnd < byte) return "literal";
        previousEnd = start + length;
      }
    }
    if (node.k === "link" || node.k === "nested_link") {
      const kind = node.k === "nested_link" || node.url.type === "page_ref" ? "page"
        : node.url.type === "block_ref" ? "block" : undefined;
      const label = node.k === "link" ? node.label ?? [] : [];
      if (kind) {
        const start = utf8ByteToUtf16Offset(source, node.span[0]);
        const end = utf8ByteToUtf16Offset(source, node.span[1]);
        const opener = kind === "page" ? "[[" : "((";
        if (source.startsWith(opener, start)) return { kind, start, end };
        // lsdoc also recognizes references used as Markdown link destinations.
        if (node.k === "link" && (node.url.type === "page_ref" || node.url.type === "block_ref")) {
          const labelEnd = label.at(-1)?.span?.[1];
          const delimiter = source.indexOf(`](${opener}`, labelEnd === undefined ? start : utf8ByteToUtf16Offset(source, labelEnd));
          const destination = delimiter + 2;
          const destinationEnd = destination + node.url.v.length + 4;
          if (delimiter >= start && destinationEnd < end && destination <= at && at < destinationEnd)
            return { kind, start: destination, end: destinationEnd };
        }
      } else if (node.k === "link") {
        const synthetic = label[0]?.k === "plain" && (label[0].text === node.full || `[[${label[0].text}]]` === node.full);
        if (synthetic || !label.some(child => child.span && child.span[0] <= byte && byte < child.span[1])) return "literal";
      }
      return ownerAt(label, source, at, byte);
    }
    if ("children" in node) return ownerAt(node.children, source, at, byte);
  }
}

function plainTail(raw: string, title: number, caret: number, close: number, format: Format): boolean {
  const tail = raw.slice(caret, close);
  if (tail && !parseInline(tail, format)?.every(node => node.k === "plain")) return false;

  // A literal that began inside the title can continue across the caret and closer.
  const literal = pairedContext(raw, raw.slice(title, caret), title, caret, LITERAL_PAIRS[format]);
  const from = literal.end > close + 2 ? literal.start : caret;
  const [, reference, alternate] = tail.includes("[[") || tail.includes("((") ? referenceHorizons(raw, caret, close) : [caret, close, close];
  // An underscore inside a Markdown identifier cannot open emphasis (lsdoc inline.rs).
  const identifier = (offset: number, token: string) => format === "md" && token === "_" && offset > caret &&
    /^[a-z0-9]_[a-z0-9]$/i.test(raw.slice(offset - 1, offset + 2));
  const horizon = Math.max(reference, alternate, contextHorizon(raw, caret, close, format, true, identifier), literal.end);
  if (horizon === close) return true;
  const source = raw.slice(from, horizon);
  const right = utf16ToUtf8ByteOffset(source, close - from);
  const left = utf16ToUtf8ByteOffset(source, caret - from);
  // A construct can need source beyond the title to close. Only its text on
  // the right of the caret affects the pick.
  return parseInline(source, format)?.every(node => node.k === "plain" ||
    (node.span && (node.span[0] >= right || node.span[1] <= left))) ?? false;
}

/** The shared picker and edit range. Parse through candidate terminators, then ask
 * whether the title text on the right of the caret is plain (D-20261119-P853C). */
export function referenceCompletion(raw: string, before: string, lineStart: number, caret: number, format: Format): Trigger | null {
  const marker = Math.max(before.lastIndexOf("[["), before.lastIndexOf("(("));
  const hash = before.lastIndexOf("#");
  const tag = hash >= 0 && (hash === 0 || /\s/u.test(before[hash - 1])) && isBareTagPrefix(before.slice(hash + 1));
  if (marker === -1 && !tag) return null;

  const ready = parserReady();
  const at = before.length;
  let source = before;
  let nodes: Inline[] = [];
  if (ready) {
    if (multilineLiteralAt(raw, caret, format)) return null;
    const [opening, reference, alternate] = marker >= 0 ? referenceHorizons(raw, lineStart, caret) : [caret, caret, caret];
    source = raw.slice(lineStart, reference);
    const parsed = parseInline(source, format);
    if (!parsed) return null;
    nodes = parsed;
    let enclosing = ownerAt(nodes, source, at);
    const extra = !enclosing || (typeof enclosing !== "string" && enclosing.start !== opening - lineStart) ? alternate : reference;
    // A delimiter in an already parsed literal cannot open another one.
    const horizon = Math.max(reference, extra, contextHorizon(raw, lineStart, caret, format, false,
      offset => ownerAt(parsed, source, offset - lineStart) === "literal"));
    if (horizon > reference) {
      source = raw.slice(lineStart, horizon);
      const extended = parseInline(source, format);
      if (!extended) return null;
      nodes = extended;
      enclosing = ownerAt(nodes, source, at);
    }
    if (enclosing === "literal" || enclosing === "macro") return null;
    if (enclosing && enclosing.start + 2 <= at && at <= enclosing.end - 2) {
      const replace = plainTail(raw, lineStart + enclosing.start + 2, caret, lineStart + enclosing.end - 2, format);
      const end = replace && enclosing.kind === "page" ? enclosing.end - 2 : at;
      return { kind: enclosing.kind, start: lineStart + enclosing.start, end: caret,
        query: source.slice(enclosing.start + 2, end), ...(replace ? { replaceEnd: lineStart + enclosing.end } : {}) };
    }
  }

  if (marker >= 0 && !ownerAt(nodes, source, marker)) {
    const kind = before[marker] === "[" ? "page" : "block";
    const closer = kind === "page" ? "]]" : "))";
    const query = before.slice(marker + 2);
    const start = lineStart + marker;
    // Empty editor auto-pairs are scaffolds. lsdoc requires a nonempty name.
    if (!query && raw.startsWith(closer, caret)) return { kind, query, start, end: caret, replaceEnd: caret + 2 };
    const pending = !(kind === "page" ? /[\[\]]/ : /[()]/).test(query);
    if (pending) {
      // Org retains escape bytes in plain text. Use the parser to check an unfinished escaped opener.
      if (ready && format === "org" && before[marker - 1] === "\\") {
        const sample = before.slice(0, marker + 2) + "x" + closer;
        if (!ownerAt(parseInline(sample, format) ?? [], sample, marker)) return null;
      }
      return { kind, query, start, end: caret };
    }
  }
  if (tag) {
    const owner = ownerAt(nodes, source, hash);
    if (!owner) return { kind: "tag", query: before.slice(hash + 1), start: lineStart + hash, end: caret };
  }
  return null;
}
