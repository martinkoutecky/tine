import type { Format, Inline } from "../render/ast";
import { parserReady } from "../render/parse";
import { utf16ToUtf8ByteOffset, utf8ByteToUtf16Offset } from "../render/spans";
import { parse_inline_json } from "../render/wasm/lsdoc_wasm.js";
import { isBareTagPrefix } from "../tags";
import type { Trigger } from "./autocomplete";

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

function closedAt(nodes: Inline[], marker: number): Inline | undefined {
  for (const node of nodes) {
    if (!node.span || marker < node.span[0] || marker >= node.span[1]) continue;
    if (referenceKind(node) || node.k === "macro") return node;
    const children = inlineChildren(node);
    if (children) return closedAt(children, marker);
  }
}

function startsReferenceBefore(nodes: Inline[], boundary: number): boolean {
  return nodes.some(node => node.span && node.span[0] < boundary && (
    referenceKind(node) || startsReferenceBefore(inlineChildren(node) ?? [], boundary)
  ));
}

/** lsdoc allows a plain page or block ref to absorb another opener when the
 * first ref is unfinished. Parse that later text on its own before agreeing
 * to replace it. A genuine nested page ref has its own parser node. */
function referenceEnd(raw: string, start: number, caret: number, kind: "page" | "block", format: Format,
  nestedOnly = false): number {
  const closer = kind === "page" ? "]]" : "))";
  // Empty editor auto-pairs are scaffolds. lsdoc requires a nonempty name.
  if (!nestedOnly && caret === start + 2 && raw.startsWith(closer, caret)) return caret + 2;
  if (!parserReady()) return caret;

  // Delimiters only schedule parser probes. Start with enough room to amortize
  // rereading the reference prefix and double until lsdoc resolves the span.
  let scanned = caret;
  let needsSibling = false;
  for (let size = Math.max(32, caret - start); ; size *= 2) {
    const limit = Math.min(caret + size, raw.length);
    const from = Math.max(start + 2, scanned - 1);
    const chunk = raw.slice(from, limit);
    const newline = chunk.search(/[\r\n]/);
    const done = newline !== -1 || limit === raw.length;
    const end = newline === -1 ? limit : from + newline;
    scanned = limit;
    const lineChunk = newline === -1 ? chunk : chunk.slice(0, newline);
    if (!lineChunk.includes(closer) && !(needsSibling && (done || /\]\]|\)\)/.test(lineChunk)))) {
      if (done) return caret;
      continue;
    }

    const sample = raw.slice(start, end);
    const owner = parseInline(sample, format)[0];
    if (!owner?.span || owner.span[0] !== 0 || referenceKind(owner) !== kind) {
      if (done) return caret;
      continue;
    }
    const replaceEnd = start + utf8ByteToUtf16Offset(sample, owner.span[1]);
    if (replaceEnd <= caret) return caret;
    if (owner.k === "nested_link") return replaceEnd;
    const later = raw.slice(caret, replaceEnd);
    const mayNest = kind === "page" && later.includes("[[");
    needsSibling = later.includes("[[") || (later.includes("((") && (kind === "block" || later.includes(",")));
    if (!needsSibling) return nestedOnly ? caret : replaceEnd;
    if (done || !mayNest) {
      const suffix = raw.slice(caret, end);
      const boundary = utf16ToUtf8ByteOffset(suffix, later.length);
      if (startsReferenceBefore(parseInline(suffix, format), boundary)) return caret;
      if (done) return nestedOnly ? caret : replaceEnd;
    }
  }
}

/** Owns the active reference marker, its query and its replacement span.
 * Reads the current line prefix and, for a page/block pick, only as far right
 * as needed to resolve its span. No graph or editor state is consulted. */
export function referenceCompletion(raw: string, before: string, lineStart: number, caret: number, format: Format): Trigger | null {
  const page = before.lastIndexOf("[[");
  const block = before.lastIndexOf("((");
  const hash = before.lastIndexOf("#");
  const tag = hash >= 0 && (hash === 0 || /\s/u.test(before[hash - 1])) && isBareTagPrefix(before.slice(hash + 1));
  const marker = Math.max(page, block);
  if (marker === -1 && !tag) return null;

  let prefix: Inline[] | undefined;
  const closed = (at: number) => {
    if (!parserReady() || !/\]\]|\)\)|\}\}/.test(before.slice(at))) return undefined;
    prefix ??= parseInline(before, format);
    const marker = utf16ToUtf8ByteOffset(before, at);
    const owner = closedAt(prefix, marker);
    if (owner || !before.endsWith("))") || raw[caret] !== ")") return owner;
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
    if (!(tag && hash > marker) && ended?.k === "link" && ended.url.type === "page_ref" && ended.span) {
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
