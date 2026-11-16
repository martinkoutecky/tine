// Thread membership is shared by body structure, navigation and pointer paths.
// OG sees ordinary child bullets on disk; in Tine every descendant of a comment
// belongs to that annotation, even when an external editor nested it by hand.
import { doc, formatForBlock } from "./model";
import { isCommentBlock } from "../comments";
import { facetsOf } from "../render/facets";
import type { Format } from "../types";

// Screen only the key; the parser owns which separators/layout are accepted.
const MAY_QUOTE = /tine\.quote/i;

/** A quoted comment block, as distinct from an unquoted reply in its thread. */
export function isCommentId(id: string, format: Format = formatForBlock(id)): boolean {
  const n = doc.byId[id];
  return !!n && MAY_QUOTE.test(n.raw) && isCommentBlock(facetsOf(n.raw, format).properties, n.parent !== null);
}

/** The comment whose thread holds `id`: itself or its outermost quoted comment
 * ancestor (DESIGN §2: a quoted reply stays in its root's thread), or null for
 * an ordinary block. This is the one predicate for thread membership. O(depth). */
export function threadRootOf(id: string, format: Format = formatForBlock(id)): string | null {
  let root: string | null = null;
  for (let at: string | null = id; at !== null; at = doc.byId[at]?.parent ?? null) {
    if (isCommentId(at, format)) root = at;
  }
  return root;
}
