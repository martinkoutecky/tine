export type BlockDropPosition = "before" | "after" | "child";

interface RectLike {
  left: number;
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
): BlockDropPosition {
  if (clientX - blockRect.left > 50) return "child";
  return clientY < mainRect.top + mainRect.height / 2 ? "before" : "after";
}
