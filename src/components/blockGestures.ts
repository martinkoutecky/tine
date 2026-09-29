/** Pointer gestures on a rendered block, split out of Block.tsx: the bullet's
 * drag-to-reorder and the click-or-drag gesture on rendered content.
 *
 * - `renderedClickOffset(...)` maps a click on rendered content to the raw
 *   offset the editor should open at, or null when no trustworthy mapping exists.
 * - `beginDrag(id, e)` arms a bullet drag from a mousedown. Past a 4 px
 *   threshold it ends editing, tracks a drop indicator (`dropInd`) under the
 *   pointer and, on mouseup, moves the active selection (or just the block) with
 *   one `moveBlocksRelative` call. The move is refused when the graph changed
 *   during the drag (binding check) or when the target is inside a moved subtree.
 * - `beginEditGesture(...)` resolves a rendered-content mousedown at mouseup:
 *   a click starts editing at the captured offset; a drag that crosses into another
 *   block escalates to block selection. Callers need not know the listeners. */
import { createSignal } from "solid-js";
import { captureBinding, stillBound } from "../binding";
import { clearSelection, extendSelectionTo, moveBlocksRelative, selectBlock, selectedIds, node as docNode, type OutlineScope } from "../document";
import { endEdit, startEditing } from "../editorController";
import { dropSelection, setDragSelectionSuppressed } from "../dragSelectionGuard";
import { codeBodyProjection } from "../editor/codeFence";
import { isBuiltinHidden, splitProps } from "../editor/properties";
import { clickBeyondRenderedEnd, codeCardOffsetFromRange, editorOffsetFromRenderedRange } from "../render/spans";


// Pointer-based drag reorder (HTML5 DnD is unreliable in WebKitGTK).
const [dragId, setDragId] = createSignal<string | null>(null);
const [dropInd, setDropInd] = createSignal<{ id: string; before: boolean } | null>(null);
let dragMoved = false;

/** True from the drag threshold until the tick after mouseup: the bullet's click
 *  handler reads it to tell a finished drag from a click. */
export function bulletDragMoved(): boolean {
  return dragMoved;
}

/** The block being dragged by its bullet, and the current drop indicator. */
export { dragId, dropInd };

export function beginDrag(id: string, e: MouseEvent) {
  const binding = captureBinding(), startX = e.clientX;
  const startY = e.clientY;
  let capturedIds: string[] | null = null;
  dragMoved = false;
  const onMove = (ev: MouseEvent) => {
    if (!dragMoved && Math.hypot(ev.clientX - startX, ev.clientY - startY) < 4) return;
    if (!dragMoved) {
      dragMoved = true;
      // A bullet drag moves the active selection when there is one (GH #240).
      const selected = selectedIds();
      capturedIds = selected.length ? [...selected] : [id];
      setDragId(id);
      endEdit("drag-start");
      // Moving a block is not a text gesture. WebKit otherwise runs its own
      // selection drag from the bullet and paints every block the pointer
      // crosses blue (GH #424, macOS; Chromium does not do this).
      setDragSelectionSuppressed(true);
    }
    // WebKit can re-anchor a selection mid-drag; the class alone is not enough.
    dropSelection();
    const el = (document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null)?.closest(
      ".ls-block"
    ) as HTMLElement | null;
    const tid = el?.dataset.blockId;
    if (tid) {
      const main = el!.querySelector(".block-main")!.getBoundingClientRect();
      setDropInd({ id: tid, before: ev.clientY < main.top + main.height / 2 });
    } else {
      setDropInd(null);
    }
  };
  const onUp = () => {
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
    setDragSelectionSuppressed(false);
    const ind = dropInd();
    if (stillBound(binding) && dragMoved && ind && docNode(ind.id)) {
      // One transaction: normalizes nested captures, refuses a drop into a
      // moved subtree, and persists a cross-page move as one save group.
      void moveBlocksRelative(capturedIds ?? [id], ind.id, ind.before ? "before" : "after");
    }
    setDragId(null);
    setDropInd(null);
    setTimeout(() => (dragMoved = false), 0);
  };
  document.addEventListener("mousemove", onMove);
  document.addEventListener("mouseup", onUp);
}

// --- Click / drag gesture on rendered block content -------------------------
//
// The caret offset is captured at MOUSEDOWN (before the previously-edited
// block's blur reflows the layout — the coordinates are only valid then), but
// editing starts at MOUSEUP and only for a CLICK (pointer moved < threshold).
// A drag instead selects: within the origin block it is the browser's native
// text selection of the RENDERED text (copy gives the glyphs you see); the
// moment it crosses into another block it escalates to Tine's block selection
// (muscle memory from OG — but deterministic: the escalation rule is purely
// "did the pointer enter a different block", never timing).
//
// Deliberately NOT OG's mousedown-instant-edit: that races the native
// selection against the DOM swap (the inconsistency Martin observed in OG).
const DRAG_THRESHOLD_PX = 4;

interface EditGesture {
  blockId: string;
  offset: number;
  owner: string | null;
  startX: number;
  startY: number;
  escalated: boolean;
  outlineScope: OutlineScope | null;
}

function blockIdAtPoint(x: number, y: number): string | null {
  const el = document.elementFromPoint(x, y);
  const row = el?.closest?.(".ls-block");
  return row?.getAttribute("data-block-id") ?? null;
}

/** Arm a click-or-drag gesture from a rendered-content mousedown. Document-level
 *  listeners resolve it, so post-blur layout shifts can't misroute the mouseup. */
export function beginEditGesture(
  e: MouseEvent,
  blockId: string,
  offset: number,
  owner: string | null,
  outlineScope: OutlineScope | null,
): void {
  clearSelection(); // a plain gesture replaces any active block selection (shift-click returns before this)
  const g: EditGesture = { blockId, offset, owner, startX: e.clientX, startY: e.clientY, escalated: false, outlineScope };
  const onMove = (ev: MouseEvent) => {
    const moved =
      Math.abs(ev.clientX - g.startX) > DRAG_THRESHOLD_PX || Math.abs(ev.clientY - g.startY) > DRAG_THRESHOLD_PX;
    if (!moved) return;
    const over = blockIdAtPoint(ev.clientX, ev.clientY);
    if (g.escalated) {
      if (over) extendSelectionTo(over, g.outlineScope);
      return;
    }
    if (over && over !== g.blockId) {
      // Crossed into another block: escalate to block selection for the rest of
      // the gesture (never de-escalate — flipping modes mid-drag is jarring).
      g.escalated = true;
      window.getSelection()?.removeAllRanges();
      selectBlock(g.blockId, g.outlineScope);
      extendSelectionTo(over, g.outlineScope);
    }
  };
  const onUp = (ev: MouseEvent) => {
    document.removeEventListener("mousemove", onMove, true);
    document.removeEventListener("mouseup", onUp, true);
    if (g.escalated) return; // block selection stands
    const moved =
      Math.abs(ev.clientX - g.startX) > DRAG_THRESHOLD_PX || Math.abs(ev.clientY - g.startY) > DRAG_THRESHOLD_PX;
    if (moved) return; // an in-block text selection (or a stray drag) — not a click
    startEditing(g.blockId, g.offset, g.owner);
  };
  document.addEventListener("mousemove", onMove, true);
  document.addEventListener("mouseup", onUp, true);
}

/** Click on rendered block content -> raw caret offset for the editor, placing
 *  the caret WHERE you clicked when lsdoc span data can map the rendered leaf
 *  back through source bytes and hidden props. Anything without trustworthy
 *  span data (chips, macro hosts, parser fallback) returns null and the caller
 *  keeps the old end-of-block behaviour. */
export function renderedClickOffset(contentRef: HTMLElement, raw: string, fmt: "md" | "org", e: MouseEvent): number | null {
  const d = document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
  // GH #489: a whole-block code card is highlight.js markup with no span data,
  // so the mapper below always declined and the caret fell to the end of the
  // block, hundreds of lines from the click. Answer it first, from rendered text
  // position; the past-the-end rule must not run for it (a click right of a
  // SHORT line in a tall card means that line's end). Offsets leave here in
  // visible-raw coordinates; the editor's `focusNow` maps them through the
  // body-only wrapper.
  const codeProjection = codeBodyProjection(splitProps(raw, isBuiltinHidden, fmt).visible, fmt);
  if (codeProjection) {
    const codeRange = d.caretRangeFromPoint?.(e.clientX, e.clientY);
    const offset = codeRange ? codeCardOffsetFromRange(contentRef, codeRange) : null;
    return offset === null ? null : codeProjection.open.length + Math.min(offset, codeProjection.body.length);
  }
  // GH #465: a click in the empty run-out past the last glyph means "the end",
  // whatever the block ends with. Asked before the span map, because a trailing
  // construct with an invisible closing delimiter (`*italic*`) maps that click
  // to a legitimate-looking interior offset just before the delimiter.
  if (clickBeyondRenderedEnd(contentRef, e.clientX, e.clientY)) return splitProps(raw, isBuiltinHidden, fmt).visible.length;
  const range = d.caretRangeFromPoint?.(e.clientX, e.clientY);
  if (!range) return null;
  return editorOffsetFromRenderedRange(contentRef, range, raw, isBuiltinHidden, fmt);
}
