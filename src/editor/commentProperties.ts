import { quoteSelectorOf } from "../comments";
import { facetsOf } from "../render/facets";
import type { Format } from "../render/ast";
import { BUILTIN_HIDE, SHEET_CELL_HIDE, type HidePolicy } from "./properties";

interface CommentNode { raw: string; parent: string | null }
type Props = readonly (readonly [string, string])[];

/** The editor and save projection share one policy for roots and all replies.
 * The kernel's sheet-cell policy hides built-ins and every accepted tine.* row.
 * Editors capture this on entry so handwritten properties can finish before
 * reentry hides them; serialization asks against the current document tree. */
export function commentEditorHide(node: CommentNode, properties: Props, format: Format,
  readNode: (id: string) => CommentNode | undefined): HidePolicy {
  if (node.parent !== null && quoteSelectorOf(properties) !== null) return SHEET_CELL_HIDE;
  for (let id = node.parent; id !== null;) {
    const ancestor = readNode(id);
    if (!ancestor) break;
    if (ancestor.parent !== null && quoteSelectorOf(facetsOf(ancestor.raw, format).properties) !== null) return SHEET_CELL_HIDE;
    id = ancestor.parent;
  }
  return BUILTIN_HIDE;
}
