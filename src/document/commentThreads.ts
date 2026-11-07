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

/** The nearest quoted comment containing `id`, including itself, or null.
 * This is the one predicate for whether a block belongs to a comment thread. */
export function threadRootOf(id: string, format: Format = formatForBlock(id)): string | null {
  for (let at: string | null = id; at !== null; at = doc.byId[at]?.parent ?? null) {
    if (isCommentId(at, format)) return at;
  }
  return null;
}
