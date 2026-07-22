import { isPropertyLine } from "../render/block";
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

function removeTags(text: string): string {
  return text
    .replace(/#\[\[[^\]]*\]\]/g, "")
    .replace(/(^|\s)#[\w/-]+/g, "$1")
    .replace(/[ \t]{2,}/g, " ")
    .trimEnd();
}

function removeEmphasis(text: string, format: Format): string {
  if (format === "org") {
    return text.replace(/(^|\s)([*\/_+])([^\n]+?)\2(?=$|[\s.,!?;:])/g, "$1$3");
  }
  return text
    .replace(/(\*\*|__)(.*?)\1/g, "$2")
    .replace(/(\*|_)(.*?)\1/g, "$2")
    .replace(/~~(.*?)~~/g, "$1")
    .replace(/==(.*?)==/g, "$1");
}

/** Apply the three transformations shared by Text, OPML, and HTML. */
export function cleanInline(text: string, format: Format, options: MarkupExportOptions): string {
  let result = text;
  if (options.removeTags) result = removeTags(result);
  if (options.stripLinks) result = result.replace(/\[\[([^\]]*)\]\]/g, "$1");
  if (options.removeEmphasis) result = removeEmphasis(result, format);
  return result;
}

/** Property AST nodes are absent from OG's OPML/HTML output. Mirror that for
 * Markdown property lines and Org property drawers before serializing a node. */
export function nodeText(node: ExportNode, options: MarkupExportOptions): string {
  const kept: string[] = [];
  let inOrgProperties = false;
  for (const line of node.raw.split("\n")) {
    if (/^\s*:PROPERTIES:\s*$/i.test(line)) {
      inOrgProperties = true;
      continue;
    }
    if (inOrgProperties) {
      if (/^\s*:END:\s*$/i.test(line)) inOrgProperties = false;
      continue;
    }
    if (!isPropertyLine(line)) kept.push(cleanInline(line, node.format ?? "md", options));
  }
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

interface HtmlEmphasisPattern {
  regex: RegExp;
  tag: "strong" | "em" | "u" | "del" | "mark";
  contentGroup: number;
  prefixGroup?: number;
}

const MARKDOWN_EMPHASIS: HtmlEmphasisPattern[] = [
  { regex: /(\*\*|__)(.+?)\1/g, tag: "strong", contentGroup: 2 },
  { regex: /~~(.+?)~~/g, tag: "del", contentGroup: 1 },
  { regex: /==(.+?)==/g, tag: "mark", contentGroup: 1 },
  { regex: /(\*|_)(.+?)\1/g, tag: "em", contentGroup: 2 },
];

const ORG_EMPHASIS: HtmlEmphasisPattern[] = [
  { regex: /(^|[\s([{])\*([^*\n]+?)\*(?=$|[\s)\]},.!?;:])/g, tag: "strong", contentGroup: 2, prefixGroup: 1 },
  { regex: /(^|[\s([{])\/([^/\n]+?)\/(?=$|[\s)\]},.!?;:])/g, tag: "em", contentGroup: 2, prefixGroup: 1 },
  { regex: /(^|[\s([{])_([^_\n]+?)_(?=$|[\s)\]},.!?;:])/g, tag: "u", contentGroup: 2, prefixGroup: 1 },
  { regex: /(^|[\s([{])\+([^+\n]+?)\+(?=$|[\s)\]},.!?;:])/g, tag: "del", contentGroup: 2, prefixGroup: 1 },
];

function renderEmphasis(text: string, patterns: HtmlEmphasisPattern[], index = 0): string {
  if (index >= patterns.length) return escapeXmlText(text);
  const pattern = patterns[index];
  pattern.regex.lastIndex = 0;
  let cursor = 0;
  let result = "";
  for (const match of text.matchAll(pattern.regex)) {
    const start = match.index ?? 0;
    result += renderEmphasis(text.slice(cursor, start), patterns, index + 1);
    if (pattern.prefixGroup) result += escapeXmlText(match[pattern.prefixGroup] ?? "");
    result += `<${pattern.tag}>${renderEmphasis(match[pattern.contentGroup] ?? "", patterns, index + 1)}</${pattern.tag}>`;
    cursor = start + match[0].length;
  }
  return result + renderEmphasis(text.slice(cursor), patterns, index + 1);
}

/** Render only serializer-owned emphasis tags; all graph text is escaped. */
export function renderHtmlInline(text: string, format: Format, removeMarkers: boolean): string {
  if (removeMarkers) return escapeXmlText(text);
  const patterns = format === "org" ? ORG_EMPHASIS : MARKDOWN_EMPHASIS;
  return renderEmphasis(text, patterns);
}
