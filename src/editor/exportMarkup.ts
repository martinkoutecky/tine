import { editBlock, parseBlock } from "../render/parse";
import type { Format } from "../render/ast";
import type { ExportNode, MaxDepth } from "./exportText";

export interface MarkupExportOptions {
  stripLinks: boolean;
  removeEmphasis: boolean;
  removeTags: boolean;
  maxDepth?: MaxDepth;
}

/** Roots are level 1, matching OG's shared `keep-only-level<=n` transform. */
export function includesChildren(level: number, maxDepth: MaxDepth | undefined): boolean {
  return maxDepth === undefined || maxDepth === "all" || level < maxDepth;
}

/** Export cleanup is a policy over accepted AST spans. Literal nodes are never
 * traversed. Source mode retains all unselected bytes; HTML escapes graph text
 * and adds only serializer-owned emphasis tags. O(source bytes + AST nodes),
 * one cached block parse, no regex recognition or per-line reparsing. */
export function cleanInline(text: string, format: Format, options: MarkupExportOptions, html = false): string {
  if (!html && !options.removeTags && !options.stripLinks && !options.removeEmphasis) return text;
  const patches: { start: number; end: number; text: string; tag?: boolean }[] = [];
  const lead = new TextEncoder().encode(text.slice(0, text.length - text.trimStart().length)).length;
  const range = (span: readonly number[]) => ({start:span[0] - 2 + lead, end:span[1] - 2 + lead});
  const escaped = (s: string) => html ? escapeXmlText(s) : s;
  const visit = (node: unknown): void => {
    if (Array.isArray(node)) { node.forEach(visit); return; }
    if (!node || typeof node !== "object") return;
    const n = node as { k?: string; kind?: string; span?: number[]; children?: unknown[]; emph?: string; url?: {type: string; v?: string}; label?: unknown[] };
    // These accepted nodes own their complete literal contents.
    if (["code", "verbatim", "latex", "inline_html"].includes(n.k ?? "") || ["src", "example", "raw_html", "properties"].includes(n.kind ?? "")) return;
    if (n.span && n.k) {
      const r = range(n.span);
      if (r.start === undefined || r.end === undefined) return;
      if (n.k === "tag" && options.removeTags) {
        patches.push({...r, text:"", tag:true}); return;
      }
      if (n.k === "link" && options.stripLinks && n.url?.type === "page_ref" && !n.label?.length) {
        patches.push({...r, text:escaped(n.url.v ?? "")}); return;
      }
      if (n.k === "emphasis" && (options.removeEmphasis || html)) {
        const children = n.children as {span?: number[]}[];
        const first = children?.[0]?.span;
        const last = children?.[children.length - 1]?.span;
        if (first && last) {
          const tag = ({Bold:"strong", Italic:"em", Underline:"u", Strike_through:"del", Highlight:"mark"} as Record<string,string>)[n.emph!];
          patches.push({start:r.start, end:range(first).start, text:html && !options.removeEmphasis ? `<${tag}>` : ""});
          patches.push({start:range(last).end, end:r.end, text:html && !options.removeEmphasis ? `</${tag}>` : ""});
        }
      }
    }
    for (const [key, value] of Object.entries(node)) if (key !== "span" && key !== "span_map") visit(value);
  };
  visit(parseBlock(text, format === "org"));
  patches.sort((a,b) => a.start - b.start || a.end - b.end);
  // Convert sorted byte coordinates in one forward pass; no per-character map
  // and no repeated encoding/decoding for individual spans.
  let byte = 0, unit = 0;
  const toUnits = (target:number) => {
    while (byte < target && unit < text.length) {
      const cp = text.codePointAt(unit)!;
      byte += cp <= 0x7f ? 1 : cp <= 0x7ff ? 2 : cp <= 0xffff ? 3 : 4;
      unit += cp > 0xffff ? 2 : 1;
    }
    return unit;
  };
  let out = "", at = 0;
  for (const p of patches) {
    let start = toUnits(p.start);
    const end = toUnits(p.end);
    // Named presentation policy: tidy one space adjoining a removed tag.
    if (p.tag && text[start - 1] === " " && (text[end] === " " || end === text.length || text[end] === "\n" || text[end] === "\r")) start--;
    if (start < at) continue;
    out += escaped(text.slice(at,start)) + p.text;
    at = end;
  }
  return out + escaped(text.slice(at));
}

/** Parser-owned visible body for OPML/HTML: canonical metadata is omitted,
 * literals and Org body drawers remain. O(block bytes), no file I/O; parser
 * refusal propagates so export cannot silently discard content. */
export function nodeText(node: ExportNode, options: MarkupExportOptions): string {
  const format = node.format ?? "md";
  const kept = cleanInline(editBlock(node.raw, format, { kind: "visible" }), format, options).split("\n");
  while (kept.length > 1 && kept[kept.length - 1].trim() === "") kept.pop();
  return kept.join("\n");
}

export function escapeXmlText(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

export function escapeXmlAttribute(text: string): string {
  return escapeXmlText(text)
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;")
    .replace(/\t/g, "&#9;")
    .replace(/\r\n?|\n/g, "&#10;");
}

/** HTML serialization shares the accepted-span cleanup policy. */
export function renderHtmlInline(text: string, format: Format, removeMarkers: boolean): string {
  return cleanInline(text, format, {stripLinks:false, removeTags:false, removeEmphasis:removeMarkers}, true);
}

export function nodeHtml(node: ExportNode, options: MarkupExportOptions): string {
  const format = node.format ?? "md";
  return cleanInline(editBlock(node.raw, format, {kind:"visible"}), format, options, true).trimEnd().split("\n").join("<br>\n");
}
