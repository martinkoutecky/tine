import { anchorQuote, quoteSelectorOf } from "../comments";
import { formatForBlock, node } from "../document";
import { BUILTIN_HIDE, rawOffsetToVisibleOffset } from "../editor/properties";
import { facetsOf } from "../render/facets";
import { renderedPointFromEditorOffset } from "../render/spans";

/** Revealed resolved threads have no highlight element to measure. Reuse the
 * editor's exact source mapping to measure their passage without painting one.
 * O(parent bytes + rendered span leaves), only for revealed resolved cards. */
export function resolvedCommentAnchorTop(id: string, parentId: string, row: HTMLElement): number | null {
  const parent = node(parentId), comment = node(id), format = formatForBlock(parentId);
  const selector = comment && quoteSelectorOf(facetsOf(comment.raw, format).properties);
  const content = row.querySelector(".block-content");
  if (!parent || !selector || !content) return null;
  const anchor = anchorQuote(parent.raw, selector, format);
  if (anchor.kind !== "range") return null;
  const point = renderedPointFromEditorOffset(content, parent.raw, BUILTIN_HIDE, format,
    rawOffsetToVisibleOffset(parent.raw, anchor.start, BUILTIN_HIDE, format), "start");
  if (!point) return null;
  const range = row.ownerDocument.createRange();
  range.setStart(point.node, point.offset);
  range.collapse(true);
  const rect = range.getClientRects()[0];
  return rect && rect.height > 0 ? rect.top : null;
}
