// Export-only projection. The existing Rust-backed discriminator owns comment
// identity; carry thread membership down the already-required O(blocks) walk.
// No I/O, document edits, ancestor scans or per-edit work.
import { COMMENT_QUOTE_KEY, isCommentBlock, isCommentPresentationKey, authorOf, commentDateOf } from "../comments";
import { blockRegions, editBlock } from "../render/parse";
import { t } from "../i18n";
import type { ExportNode } from "./exportText";

export function projectExportNodes(nodes: ExportNode[], includeComments = true, inComment = false): ExportNode[] {
  const walk = (forest: ExportNode[], parent: boolean, thread: boolean): ExportNode[] => forest.flatMap(node => {
    const properties = blockRegions(node.raw, node.format ?? "md").properties.filter(p => p.primary).map(p => [p.key, p.value] as const);
    const sourceComment = isCommentBlock(properties, true);
    const comment = thread || node.comment !== undefined || ((node.hasParent ?? parent) && sourceComment);
    if (comment && !includeComments) return [];
    return [{ ...node,
      // A detached reply has membership but no source carrier. Reuse the
      // whole-parent quote so expansion/copy keeps it marked without DTO bits.
      raw: comment && !thread && !sourceComment
        ? editBlock(node.raw, node.format ?? "md", { kind: "property", key: COMMENT_QUOTE_KEY, value: "" }) : node.raw,
      ...(comment ? { comment: { author: authorOf(properties), date: commentDateOf(properties) } } : {}),
      children: walk(node.children, true, comment),
    }];
  });
  return walk(nodes, false, inComment);
}

export function exportCommentLabel(node: ExportNode): string {
  return node.comment ? [t("export-comment"), node.comment.author, node.comment.date].filter(Boolean).join(" · ") : "";
}

/** Source keeps the parser-owned comment properties even with Remove properties,
 * so a Markdown/Org export still reads back as a comment thread. */
export function exportSourceRaw(node: ExportNode, removeProperties: boolean): string {
  if (!removeProperties) return node.raw;
  const format = node.format ?? "md";
  let raw = editBlock(node.raw, format, { kind: "visible" });
  if (node.comment) for (const p of blockRegions(node.raw, format).properties) {
    if (p.primary && isCommentPresentationKey(p.key, true)) raw = editBlock(raw, format, { kind: "property", key: p.key, value: p.value });
  }
  return raw;
}
