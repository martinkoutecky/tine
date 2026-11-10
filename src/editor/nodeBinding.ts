import { node as docNode } from "../document";

/** The node a block row renders and its editor writes (GH #745, #906). Render
 * facts read the live slot, falling back to the node captured at mount while
 * that slot is absent: the flush that deletes or merges the block can evaluate
 * row and editor facts before their owners unmount.
 *
 * Writes need a live slot. A still-mounted editor keeps writing to a slot that
 * was replaced in place (undo/redo restores snapshots without unmounting it).
 * After unmount it writes only to the node it mounted: an external reload can
 * reuse runtime ids for different nodes, and Chromium-family WebViews deliver
 * blur after removing the old focused textarea; that old buffer must neither
 * overwrite the replacement nor close its carried editor
 * (UI-CARRY-RELOAD-STALE-BLUR). O(1), owner-local; no graph mutation. */
export function blockNodeBinding(id: () => string, mounted: () => boolean = () => false) {
  const shownNode = docNode(id());
  const live = () => docNode(id());
  return {
    shownNode,
    live,
    node: () => live() ?? shownNode,
    current: () => live() === shownNode,
    writable: () => {
      const n = live();
      return !!n && (mounted() || n === shownNode);
    },
  };
}
