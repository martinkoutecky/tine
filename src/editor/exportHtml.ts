import type { ExportNode } from "./exportText";
import {
  includesChildren,
  nodeText,
  renderHtmlInline,
  type MarkupExportOptions,
} from "./exportMarkup";

function listItem(node: ExportNode, options: MarkupExportOptions, level: number, indent: string): string[] {
  const text = nodeText(node, options)
    .split("\n")
    .map((line) => renderHtmlInline(line, node.format ?? "md", options.removeEmphasis))
    .join("<br>\n");
  const children = includesChildren(level, options.maxDepth) ? node.children : [];
  if (!children.length) return [`${indent}<li>${text}</li>`];
  return [
    `${indent}<li>${text}`,
    `${indent}  <ul>`,
    ...children.flatMap((child) => listItem(child, options, level + 1, `${indent}    `)),
    `${indent}  </ul>`,
    `${indent}</li>`,
  ];
}

/** Pure ExportNode-forest → nested HTML fragment serialization.
 *
 * This intentionally has no dependency on publish_html. OG 1.0.0 removes
 * Properties AST nodes and applies depth in handler/export/html.cljs:386-392,
 * accepts block or page forests at html.cljs:417-429, and exposes cleanup +
 * depth (not Text's indentation/property/newline controls) at
 * components/export.cljs:207-238,262-275.
 */
export function exportHtml(nodes: ExportNode[], options: MarkupExportOptions): string {
  return [
    "<ul>",
    ...nodes.flatMap((node) => listItem(node, options, 1, "  ")),
    "</ul>",
  ].join("\n");
}
