// **Reordering a condition inside the query sheet (SPEC §7.4, P6).** The pointer loop itself is NOT here: …
import { registerTransientLayer } from "../transientLayers";
import { beginRowReorderDrag } from "./rowReorder";

/** The live drop position: which list, which sibling, and which side of it. */
export interface QuerySheetDropTarget {
  /** `data-qs-parent` of the list — a drop indicator in one list must never be
  *  drawn by an item that happens to share an index in another. */
  parent: string;
  index: number;
  before: boolean;
}

/** The selector that matches the items of EVERY list in the sheet (GH #619 item 6: a condition can be dragged
*  into another group). Each item carries its own list's `data-qs-parent`, and the nearest one under the pointer
*  wins, so a row of a nested group answers for itself and the group's own item answers for its header. */
export const QUERY_SHEET_ITEM_SELECTOR = "[data-qs-parent]";

/** The loc a `data-qs-parent` key names (`""` is the root list). */
export function parseLocKey(key: string): number[] {
  return key === "" ? [] : key.split(".").map(Number);
}

/** Is the list `parent` the node `source` or somewhere inside it? A node cannot be dropped into its own subtree. */
export function isInsideSubtree(parent: string, source: string): boolean {
  return parent === source || parent.startsWith(`${source}.`);
}

let cancelInFlight: (() => void) | null = null;

/** Abandon the drag in flight, if any, without applying it. */
export function cancelQuerySheetReorder(): void {
  cancelInFlight?.();
}

export interface QuerySheetReorderRequest {
  /** `data-qs-parent` of the list the dragged item is in. */
  parent: string;
  from: number;
  /** Is the captured tree still the one on screen? */
  isCurrent: () => boolean;
  setTarget: (target: QuerySheetDropTarget | null) => void;
  /** The index the dragged item ends at, in its own list. */
  commit: (to: number) => void;
  /** Dropped into ANOTHER list: `parent` is that list's key and `slot` the position among its current items. */
  commitAcross: (parent: string, slot: number) => void;
}

/** The index the dragged item ends at if dropped on `target` (its own index when the drop is a no-op). */
function endsAt(from: number, target: { index: number; before: boolean }): number {
  const to = target.index + (target.before ? 0 : 1);
  return from < to ? to - 1 : to;
}

/** Start a reorder drag from an item's HANDLE. */
export function beginQuerySheetReorder(event: PointerEvent, request: QuerySheetReorderRequest): void {
  if (event.button !== 0) return;
  cancelQuerySheetReorder();

  let finished = false;
  const finish = () => {
    if (finished) return;
    finished = true;
    if (cancelInFlight === cancel) cancelInFlight = null;
    unregister();
    document.removeEventListener("pointerup", finish);
    document.removeEventListener("pointercancel", finish);
    document.removeEventListener("lostpointercapture", cancel);
    request.setTarget(null);
  };
  const cancel = () => {
    if (finished) return;
    // The same event a real cancellation sends, so the shared loop drops its own listeners and releases the …
    document.dispatchEvent(new Event("pointercancel"));
    finish();
  };
  const unregister = registerTransientLayer({
    id: "query-sheet-reorder",
    dismiss: () => {
      cancel();
      return true;
    },
  });
  cancelInFlight = cancel;

  const source = [...parseLocKey(request.parent), request.from].join(".");
  /** Where a pointer position lands: the item's own list, the slot in it, and whether that is a real move. */
  const resolve = (target: { index: number; before: boolean; row: HTMLElement }) => {
    const parent = target.row.dataset.qsParent;
    if (parent === undefined) return null;
    if (parent === request.parent) {
      const to = endsAt(request.from, target);
      return to === request.from ? null : { parent, to, slot: target.index + (target.before ? 0 : 1), same: true };
    }
    if (isInsideSubtree(parent, source)) return null;
    return { parent, to: 0, slot: target.index + (target.before ? 0 : 1), same: false };
  };

  beginRowReorderDrag(
    event,
    QUERY_SHEET_ITEM_SELECTOR,
    (target) => {
      // A slot next to the dragged item itself changes nothing, so it draws no bar (GH #619: with two conditions
      // the bar crept between them toward a "hidden" third, though the only real slot was above the first).
      const drop = target && resolve(target);
      request.setTarget(drop ? { parent: drop.parent, index: target!.index, before: target!.before } : null);
    },
    (target) => {
      if (finished || !request.isCurrent()) return;
      const drop = resolve(target);
      if (!drop) return;
      if (drop.same) request.commit(drop.to);
      else request.commitAcross(drop.parent, drop.slot);
    },
  );

  // Registered AFTER the shared loop, on the same target and phase, so its own `pointerup` — which is where …
  document.addEventListener("pointerup", finish);
  document.addEventListener("pointercancel", finish);
  document.addEventListener("lostpointercapture", cancel);
}
