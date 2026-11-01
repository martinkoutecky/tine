export type BlockDropPosition = "before" | "after" | "child";

interface RectLike {
  left: number;
  right?: number;
  top: number;
  height: number;
}

/** Logseq OG's outline drop intent: moving at least 50 px into the target block
 * nests under it; the shallower zone inserts before/after by vertical half. */
export function blockDropPosition(
  clientX: number,
  clientY: number,
  blockRect: RectLike,
  mainRect: RectLike,
  rtl = false,
): BlockDropPosition {
  // GH #823: in the mirrored note layout, depth grows from the right edge.
  if ((rtl ? (blockRect.right ?? blockRect.left) - clientX : clientX - blockRect.left) > 50) return "child";
  return clientY < mainRect.top + mainRect.height / 2 ? "before" : "after";
}
