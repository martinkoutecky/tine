// Table/board cell outlines keep their own bounds and stop at hosted grids.
// Within those bounds, both editors use the ordinary comment-aware tree walk.
import { blockIsGridView, nextVisible, node, prevVisible, threadRootOf } from "../document";

export const sheetFaceGridId = (id: string): string | null => blockIsGridView(id) ? id
  : node(id)?.children.find(child => blockIsGridView(child)
    && (threadRootOf(id) !== null || threadRootOf(child) === null)) ?? null;

export function sheetOutlineNeighbor(id: string, host: string, direction: -1 | 1): string | null {
  const scope = {
    roots: [host], forceExpandedRoot: host,
    collapsed: (key: string, stored: boolean) => stored || sheetFaceGridId(key) !== null,
  };
  return direction === 1 ? nextVisible(id, scope) : prevVisible(id, scope);
}
