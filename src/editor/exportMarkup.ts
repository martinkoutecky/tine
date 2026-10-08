import { editBlock, parseBlock, blockRegions, propertyValueInline } from "../render/parse";
import { isRenderHiddenProp } from "../render/block";
import type { RenderedTextOptions } from "../render/renderedText";
import type { Format } from "../render/ast";
import type { ExportNode, MaxDepth } from "./exportText";

export interface MarkupExportOptions {
  stripLinks: boolean;
  removeEmphasis: boolean;
  removeTags: boolean;
  maxDepth?: MaxDepth;
  resolveBlockRef?: RenderedTextOptions["resolveBlockRef"];
  /** Read-only embedded outlines already resolved by the caller; null keeps the literal. */
  resolveEmbed?: (name: string, args: string[]) => ExportNode[] | null;
}

/** Roots are level 1, matching OG's shared `keep-only-level<=n` transform. */
export function includesChildren(level: number, maxDepth: MaxDepth | undefined): boolean {
  return maxDepth === undefined || maxDepth === "all" || level < maxDepth;
}

/** Export cleanup is a policy over accepted AST spans. Literal nodes are never
 * traversed. Source mode retains all unselected bytes; HTML escapes graph text
 * and adds only serializer-owned emphasis tags. O(source bytes + AST nodes),
 * cached block parses, no regex recognition or per-line reparsing. Resolved
 * references keep target inlines with markup; literals and missing refs remain.
 * Render-hidden metadata is omitted independently of the cleanup toggles. */
export function cleanInline(text: string, format: Format, options: MarkupExportOptions, html = false): string {
  return cleanInlineResolved(text, format, options, html, new Set(), undefined, {remaining:2000});
}

function cleanInlineResolved(text: string, format: Format, options: MarkupExportOptions, html: boolean, refs: Set<string>, embeds: {raw:string; nodes:ExportNode[]}[] | undefined, budget: {remaining:number}): string {
  const patches: { start: number; end: number; text: string; tag?: boolean; embedded?:ExportNode[] }[] = [];
  const lead = new TextEncoder().encode(text.slice(0, text.length - text.trimStart().length)).length;
  const range = (span: readonly number[], base = lead - 2) => ({start:span[0] + base, end:span[1] + base});
  const escaped = (s: string) => html ? escapeXmlText(s) : s;
  const visit = (node: unknown, base = lead - 2): void => {
    if (Array.isArray(node)) { node.forEach(child => visit(child, base)); return; }
    if (!node || typeof node !== "object") return;
    const n = node as { k?: string; kind?: string; span?: number[]; children?: unknown[]; emph?: string; url?: {type: string; v?: string}; label?: unknown[]; name?: string; args?: string[] };
    // These accepted nodes own their complete literal contents.
    if (["code", "verbatim", "latex", "inline_html"].includes(n.k ?? "") || ["src", "example", "raw_html", "properties"].includes(n.kind ?? "")) return;
    if (n.span && n.k) {
      const r = range(n.span, base);
      if (r.start === undefined || r.end === undefined) return;
      if (n.k === "link" && n.url?.type === "block_ref" && options.resolveBlockRef) {
        const uuid = n.url.v ?? "";
        const target = refs.has(uuid) || refs.size >= 64 || budget.remaining <= 0 ? null : options.resolveBlockRef(uuid);
        if (target) {
          budget.remaining--;
          const next = new Set(refs).add(uuid);
          const body = editBlock(target.raw, target.format, {kind:"visible"});
          const first = parseBlock(body, target.format === "org")[0];
          const inline = first && "inline" in first ? first.inline : [];
          const start = inline[0]?.span?.[0], end = inline[inline.length - 1]?.span?.[1];
          const lead = new TextEncoder().encode(body.slice(0, body.length - body.trimStart().length)).length;
          const raw = start !== undefined && end !== undefined
            ? new TextDecoder().decode(new TextEncoder().encode(body).slice(start + lead - 2, end + lead - 2)) : "";
          patches.push({...r, text: cleanInlineResolved(raw, target.format, options, html, next, embeds, budget)});
        }
        return;
      }
      if (n.k === "macro" && n.name?.toLowerCase() === "embed" && embeds) {
        const nodes = options.resolveEmbed?.(n.name, n.args ?? []);
        if (nodes) patches.push({...r, text:"", embedded:nodes});
        return;
      }
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
          patches.push({start:r.start, end:range(first, base).start, text:html && !options.removeEmphasis ? `<${tag}>` : ""});
          patches.push({start:range(last, base).end, end:r.end, text:html && !options.removeEmphasis ? `</${tag}>` : ""});
        }
      }
    }
    for (const [key, value] of Object.entries(node)) if (key !== "span" && key !== "span_map") visit(value, base);
  };
  visit(parseBlock(text, format === "org"));
  for (const property of blockRegions(text, format).properties) {
    if (isRenderHiddenProp(property.key)) {
      patches.push({start:property.line[0], end:property.line[1], text:""});
      continue;
    }
    visit(propertyValueInline(property, format), property.value_range[0]);
  }
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
    if (p.embedded && embeds) { embeds.push({raw:out, nodes:p.embedded}); out = ""; }
    at = end;
  }
  return out + escaped(text.slice(at));
}

/** Shared OG ref/embed transform before destination cleanup. O(admitted source
 * bytes + expanded nodes); embed depth is five (common.cljs), with a 2000-node
 * expansion budget. Missing/cyclic targets stay literal; no I/O or graph writes.
 * Callers supply semantic resolvers and need no parser offsets. */
export function expandExportNodes(nodes: ExportNode[], options: MarkupExportOptions): ExportNode[] {
  let remaining = 2000, truncated = false;
  const refBudget = {remaining:2000};
  const walk = (forest: ExportNode[], depth: number, expanded = false): ExportNode[] => forest.flatMap(node => {
    if (expanded && remaining-- <= 0) { truncated = true; return []; }
    const policy = {...options, stripLinks:false, removeTags:false, removeEmphasis:false};
    // Resolve refs first; embeds in the resulting inlines then splice in order.
    const resolved = cleanInlineResolved(node.raw, node.format ?? "md", {
      ...policy, resolveEmbed:undefined,
    }, false, new Set(), undefined, refBudget);
    const embedded: {raw:string; nodes:ExportNode[]}[] = [];
    const tail = cleanInlineResolved(resolved, node.format ?? "md", {
      ...policy, resolveBlockRef:undefined,
      resolveEmbed: depth < 5 && remaining > 0 ? options.resolveEmbed : undefined,
    }, false, new Set(), embedded, refBudget);
    const children = walk(node.children, depth, expanded);
    if (!embedded.length) return [{...node, raw:tail, children}];
    const result: ExportNode[] = [];
    for (const segment of embedded) {
      if (segment.raw.trim()) result.push({...node, raw:segment.raw.trimEnd(), children:[]});
      result.push(...walk(segment.nodes, depth + 1, true));
    }
    if (tail.trim()) result.push({...node, raw:tail.trimStart(), children:[]});
    if (result.length) result[result.length - 1].children.push(...children);
    else result.push(...children);
    return result;
  });
  const result = walk(nodes, 0);
  if (truncated) result.push({raw:"[embed expansion truncated]", children:[]});
  return result;
}

/** Parser-owned visible body for OPML/HTML: canonical metadata is omitted,
 * literals and Org body drawers remain. O(block bytes), no file I/O; parser
 * refusal propagates so export cannot silently discard content. */
export function nodeText(node: ExportNode, options: MarkupExportOptions): string {
  const format = node.format ?? "md";
  const kept = cleanInline(editBlock(node.raw, format, { kind: "visible" }), format, {...options, resolveBlockRef:undefined}).split("\n");
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

export function nodeHtml(node: ExportNode, options: MarkupExportOptions): string {
  const format = node.format ?? "md";
  return cleanInline(editBlock(node.raw, format, {kind:"visible"}), format, {...options, resolveBlockRef:undefined}, true).trimEnd().split("\n").join("<br>\n");
}
