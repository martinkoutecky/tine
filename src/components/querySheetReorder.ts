// **Reordering a condition inside the query sheet (SPEC §7.4, P6).** The pointer loop itself is NOT here: …
import { registerTransientLayer } from "../transientLayers";
import { beginRowReorderDrag } from "./rowReorder";

/** The live drop position: which list, which sibling, and which side of it. */
export interface QuerySheetDropTarget {
  /** `data-qs-parent` of the list — a drop indicator in one list must never be drawn by an item that happens to … */
  parent: string;
  index: number;
  before: boolean;
}

/** The selector that matches the items of ONE list, and nothing else. */
export function querySheetSiblingSelector(parent: string): string {
  return `[data-qs-parent="${parent}"]`;
}

let cancelInFlight: (() => void) | null = null;

/** Abandon the drag in flight, if any, without applying it. */
export function cancelQuerySheetReorder(): void {
  cancelInFlight?.();
}

export interface QuerySheetReorderRequest {
  /** `data-qs-parent` of the list this drag may reorder. */
  parent: string;
  from: number;
  /** Is the captured tree still the one on screen? */
  isCurrent: () => boolean;
  setTarget: (target: QuerySheetDropTarget | null) => void;
  /** The index the dragged item ends at, in its own list. */
  commit: (to: number) => void;
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

  beginRowReorderDrag(
    event,
    querySheetSiblingSelector(request.parent),
    (target) =>
      request.setTarget(
        target ? { parent: request.parent, index: target.index, before: target.before } : null,
      ),
    (target) => {
      if (finished || !request.isCurrent()) return;
      const to = target.index + (target.before ? 0 : 1);
      const adjusted = request.from < to ? to - 1 : to;
      if (adjusted !== request.from) request.commit(adjusted);
    },
  );

  // Registered AFTER the shared loop, on the same target and phase, so its own `pointerup` — which is where …
  document.addEventListener("pointerup", finish);
  document.addEventListener("pointercancel", finish);
  document.addEventListener("lostpointercapture", cancel);
}
