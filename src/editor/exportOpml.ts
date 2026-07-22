import type { ExportNode } from "./exportText";
import {
  escapeXmlAttribute,
  escapeXmlText,
  includesChildren,
  nodeText,
  type MarkupExportOptions,
} from "./exportMarkup";

function outline(node: ExportNode, options: MarkupExportOptions, level: number, indent: string): string[] {
  const text = escapeXmlAttribute(nodeText(node, options));
  const children = includesChildren(level, options.maxDepth) ? node.children : [];
  if (!children.length) return [`${indent}<outline text="${text}"/>`];
  return [
    `${indent}<outline text="${text}">`,
    ...children.flatMap((child) => outline(child, options, level + 1, `${indent}  `)),
    `${indent}</outline>`,
  ];
}

/** Pure ExportNode-forest → OPML 2.0 serialization.
 *
 * OG 1.0.0 removes Properties AST nodes and applies depth before serialization
 * (src/main/frontend/handler/export/opml.cljs:404-410), accepts block or page
 * forests at opml.cljs:433-450, and exposes only cleanup + depth for OPML
 * (src/main/frontend/components/export.cljs:207-238,262-275).
 */
export function exportOpml(
  nodes: ExportNode[],
  options: MarkupExportOptions,
  title = "untitled",
): string {
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<opml version="2.0">',
    "  <head>",
    `    <title>${escapeXmlText(title)}</title>`,
    "  </head>",
    "  <body>",
    ...nodes.flatMap((node) => outline(node, options, 1, "    ")),
    "  </body>",
    "</opml>",
  ].join("\n");
}
