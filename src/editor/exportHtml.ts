import { blockTextDirection, type TextDirection } from "../render/textDirection";
import type { ExportNode } from "./exportText";
import { exportCommentLabel } from "./exportProjection";
import { escapeXmlText } from "./exportMarkup";
import {
  includesChildren,
  expandExportNodes,
  nodeHtml,
  type MarkupExportOptions,
} from "./exportMarkup";

function listItem(node: ExportNode, options: MarkupExportOptions, level: number, indent: string, inherited: TextDirection = "ltr"): string[] {
  const text = (node.comment ? `<aside class="comment"><header>${escapeXmlText(exportCommentLabel(node))}</header>${nodeHtml(node, options)}</aside>` : nodeHtml(node, options));
  const children = includesChildren(level, options.maxDepth) ? node.children : [];
  const direction = blockTextDirection(node.raw, node.format ?? "md") ?? inherited;
  const open = direction !== inherited ? `<li dir="${direction}">` : "<li>";
  if (!children.length) return [`${indent}${open}${text}</li>`];
  return [
    `${indent}${open}${text}`,
    `${indent}  <ul>`,
    ...children.flatMap((child) => listItem(child, options, level + 1, `${indent}    `, direction)),
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
    ...expandExportNodes(nodes, options).flatMap((node) => listItem(node, options, 1, "  ")),
    "</ul>",
  ].join("\n");
}
