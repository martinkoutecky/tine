// Where a block's comments show when they are drawn inline (narrow panes,
// journals, the sidebar, references): directly under the commented block, before
// its other children (vision 2026-10 §3.7 "On narrow screens it is drawn inline
// under its block"; GH #110, 2026-10-31: a comment on a block with a long
// subtree showed after all of it). The file keeps the comment where it was
// written (the last child); only the view order changes, and the visible order
// that arrow keys and range selection follow (tree.ts) uses the same function.

import { untrack } from "solid-js";
import { doc, formatForBlock } from "./model";
import { quoteSelectorOf } from "../comments";
import { facetsOf } from "../render/facets";
import { commentMarginHidden } from "../commentMarginPref";

// A cheap screen before the property parse: a comment carries `quote::` (md)
// or `:quote:` (org drawers).
const MAY_QUOTE = /quote::|:quote:/i;

function isCommentChild(id: string): boolean {
  const n = doc.byId[id];
  if (!n || n.parent === null || !MAY_QUOTE.test(n.raw)) return false;
  return quoteSelectorOf(facetsOf(n.raw, formatForBlock(id)).properties) !== null;
}

/** `children` with the comments first, each group in its stored order. Returns
 * the same array when no comment follows an ordinary child (the common case,
 * allocation-free). Reads block text untracked: a structural change re-asks, a
 * keystroke never does (visibleOrderCost.test.ts); a block edited by hand into a
 * comment moves at the next structural change, as `pageHasComment` notices it.
 * O(children) regex screens. */
export function commentsFirst(children: readonly string[]): readonly string[] {
  if (children.length < 2) return children;
  return untrack(() => {
    let seenOrdinary = false;
    let late = false;
    const comment = children.map((id) => {
      const c = isCommentChild(id);
      if (c && seenOrdinary) late = true;
      if (!c) seenOrdinary = true;
      return c;
    });
    if (!late) return children;
    return [...children.filter((_, i) => comment[i]), ...children.filter((_, i) => !comment[i])];
  });
}

/** The children the outline walk visits under a block: the comments first
 * (above), or, with the comment margin hidden (GH #845), without them, so the
 * arrow keys and range selection never land on a thread nobody can see. The
 * hidden preference is read tracked, outside the untracked block-text screens, so
 * the visible order recomputes when it is toggled. */
export function visibleChildren(children: readonly string[]): readonly string[] {
  if (!commentMarginHidden()) return commentsFirst(children);
  return untrack(() => {
    const kept = children.filter((id) => !isCommentChild(id));
    return kept.length === children.length ? children : kept;
  });
}
