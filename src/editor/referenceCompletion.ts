import { embedBlockTarget, embedPageTarget } from "../embedTarget";
import type { Format, Inline } from "../render/ast";
import { parserReady } from "../render/parse";
import { utf16ToUtf8ByteOffset } from "../render/spans";
import { utf8ToUtf16Cursor } from "../render/utf16Cursor";
import { parse_inline_json } from "../render/wasm/lsdoc_wasm.js";
import { isBareTagPrefix } from "../tags";
import type { Trigger } from "./autocomplete";
import { offsetInLiteral } from "./inlineLiteral";

type ReferenceKind = "page" | "block";
type Reference = { kind: ReferenceKind; start: number; end?: number };
type InlineOwner = Reference | "literal" | "macro";

function parseInline(source: string, format: Format): Inline[] | null {
  return JSON.parse(parse_inline_json(source, format === "org"));
}

function ownerAt(nodes: Inline[], source: string, at: number, byte = utf16ToUtf8ByteOffset(source, at)): InlineOwner | undefined {
  for (const node of nodes) {
    if (!node.span || byte < node.span[0] || byte >= node.span[1]) continue;
    if (node.k === "macro") {
      const arg = node.args.length === 1 ? node.args[0] : undefined;
      if (node.name === "embed" && arg !== undefined) {
        // The macro AST supplies the argument text. Map it back to the source.
        const offset = utf8ToUtf16Cursor(source);
        const start = source.indexOf(arg, offset(node.span[0]) + 2 + node.name.length);
        const end = start + arg.length;
        if (start >= 0 && start <= at && at <= end && end <= offset(node.span[1]) - 2) {
          if (arg.startsWith("[[")) return { kind: "page", start, end: embedPageTarget(arg) === null ? undefined : end };
          if (arg.startsWith("((")) return { kind: "block", start, end: embedBlockTarget(arg) === null ? undefined : end };
        }
      }
      return "macro";
    }
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
        const offset = utf8ToUtf16Cursor(source);
        const start = offset(node.span[0]);
        const opener = kind === "page" ? "[[" : "((";
        if (source.startsWith(opener, start)) return { kind, start, end: offset(node.span[1]) };
        // Project lsdoc's reference-valued Markdown destination back into source.
        if (node.k === "link" && (node.url.type === "page_ref" || node.url.type === "block_ref")) {
          const labelEnd = label.at(-1)?.span?.[1];
          const delimiter = source.indexOf(`](${opener}`, labelEnd === undefined ? start : offset(labelEnd));
          const destination = delimiter + 2;
          const destinationEnd = destination + node.url.v.length + 4;
          if (delimiter >= start && destination <= at && at < destinationEnd && destinationEnd < offset(node.span[1]))
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

function possibleMultilineOpener(nodes: Inline[], line: string, caret: number, format: Format): boolean {
  const at = utf8ToUtf16Cursor(line);
  const possible = (inline: Inline[]): boolean => {
    for (const node of inline) {
      if (!node.span) continue;
      const start = at(node.span[0]);
      if (start >= caret) break;
      if ("children" in node && possible(node.children)) return true;
      if (node.k !== "plain") continue;
      const text = line.slice(start, Math.min(at(node.span[1]), caret));
      if (text.includes("$$") || (format === "md" ? text.includes("``") || text.includes("~~~") : text.includes("#+"))) return true;
    }
    return false;
  };
  return possible(nodes);
}

function plainTail(line: string, title: number, caret: number, end: number, format: Format): boolean {
  const source = line.slice(title);
  const left = utf16ToUtf8ByteOffset(source, caret - title);
  const right = utf16ToUtf8ByteOffset(source, end - title);
  return parseInline(source, format)?.every(node => node.k === "plain" ||
    (node.span && (node.span[0] >= right || node.span[1] <= left))) ?? false;
}

/** The shared picker and edit range. The authored line gives the owner; the
 * independent title parse protects markup through the closer (D-20261119-P853C). */
export function referenceCompletion(raw: string, before: string, lineStart: number, caret: number, format: Format): Trigger | null {
  const marker = Math.max(before.lastIndexOf("[["), before.lastIndexOf("(("));
  const hash = before.lastIndexOf("#");
  const tag = hash >= 0 && (hash === 0 || /\s/u.test(before[hash - 1])) && isBareTagPrefix(before.slice(hash + 1));
  if (marker === -1 && !tag) return null;

  const newline = raw.indexOf("\n", caret);
  const lineEnd = newline < 0 ? raw.length : newline - Number(raw[newline - 1] === "\r");
  const line = raw.slice(lineStart, lineEnd);
  const at = before.length;
  const nodes = parserReady() ? parseInline(line, format) : [];
  if (!nodes) return null;
  const enclosing = ownerAt(nodes, line, at);
  if (enclosing === "literal" || enclosing === "macro") return null;
  const spansLines = lineStart > 0 || (newline >= 0 && possibleMultilineOpener(nodes, line, at, format));
  if (spansLines && offsetInLiteral(raw, format, caret)) return null;

  if (enclosing?.end !== undefined && enclosing.start + 2 <= at && at <= enclosing.end - 2) {
    const title = enclosing.start + 2;
    const replace = plainTail(line, title, at, enclosing.end, format);
    const queryEnd = replace && enclosing.kind === "page" ? enclosing.end - 2 : at;
    return { kind: enclosing.kind, start: lineStart + enclosing.start, end: caret,
      query: line.slice(title, queryEnd), ...(replace ? { replaceEnd: lineStart + enclosing.end } : {}) };
  }

  const pendingEmbed = enclosing && enclosing.end === undefined && marker >= enclosing.start;
  if (marker >= 0 && (pendingEmbed || !ownerAt(nodes, line, marker))) {
    const kind = before[marker] === "[" ? "page" : "block";
    const closer = kind === "page" ? "]]" : "))";
    const query = before.slice(marker + 2);
    const start = lineStart + marker;
    if (!(kind === "page" ? /[\[\]]/ : /[()]/).test(query)) {
      // Org retains escape bytes in plain nodes without a span map. An
      // escaped marker is ineligible as a pending editor trigger.
      if (format === "org" && before[marker - 1] === "\\") {
        let escape = marker - 1;
        while (before[escape - 1] === "\\") escape--;
        if ((marker - escape) % 2 !== 0) return null;
      }
      // Empty editor auto-pairs are scaffolds. lsdoc requires a nonempty name.
      const scaffold = !query && line.startsWith(closer, at);
      return { kind, query, start, end: caret, ...(scaffold ? { replaceEnd: caret + 2 } : {}) };
    }
  }
  if (tag && !ownerAt(nodes, line, hash)) {
    return { kind: "tag", query: before.slice(hash + 1), start: lineStart + hash, end: caret };
  }
  return null;
}
