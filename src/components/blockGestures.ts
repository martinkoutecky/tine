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
 * - `beginEditGesture(...)` starts editing at the captured offset on mousedown
 *   (OG, GH #368); a drag inside the block selects editor text, and one that
 *   crosses into another block escalates to block selection. Callers need not
 *   know the listeners. */
import { createSignal } from "solid-js";
import { captureBinding, bindingCurrent } from "../binding";
import { clearSelection, extendSelectionTo, moveBlocksRelative, selectBlock, selectedIds, node as docNode, type OutlineScope } from "../document";
import { endEdit, startEditing } from "../editorController";
import { dropSelection, setDragSelectionSuppressed } from "../dragSelectionGuard";
import { codeFences } from "../editor/fences";
import { codeBodyProjection } from "../editor/codeFence";
import { blockDropPosition, type BlockDropPosition } from "../editor/blockDrag";
import { textareaCaretPoints } from "../editor/caretRows";
import { isBuiltinHidden, splitProps } from "../editor/properties";
import { clickBeyondRenderedEnd, codeCardOffsetFromRange, editorOffsetFromRenderedRange } from "../render/spans";
import { documentOf, isElementTag } from "../windowRealm";


// Pointer-based drag reorder (HTML5 DnD is unreliable in WebKitGTK).
const [dragId, setDragId] = createSignal<string | null>(null);
const [dropInd, setDropInd] = createSignal<{ id: string; position: BlockDropPosition } | null>(null);
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
  // The drag's own window: listeners and hit tests follow the pointer's document.
  const doc = documentOf(e);
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
      setDragSelectionSuppressed(true, doc);
    }
    // WebKit can re-anchor a selection mid-drag; the class alone is not enough.
    dropSelection(doc);
    const el = (doc.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null)?.closest(
      ".ls-block"
    ) as HTMLElement | null;
    const tid = el?.dataset.blockId;
    if (tid) {
      const main = el!.querySelector(".block-main")!.getBoundingClientRect();
      setDropInd({
        id: tid,
        position: blockDropPosition(ev.clientX, ev.clientY, el!.getBoundingClientRect(), main),
      });
    } else {
      setDropInd(null);
    }
  };
  const onUp = () => {
    doc.removeEventListener("mousemove", onMove);
    doc.removeEventListener("mouseup", onUp);
    setDragSelectionSuppressed(false);
    const ind = dropInd();
    if (bindingCurrent(binding) && dragMoved && ind && docNode(ind.id)) {
      // One transaction: normalizes nested captures, refuses a drop into a
      // moved subtree, and persists a cross-page move as one save group.
      void moveBlocksRelative(capturedIds ?? [id], ind.id, ind.position);
    }
    setDragId(null);
    setDropInd(null);
    setTimeout(() => (dragMoved = false), 0);
  };
  doc.addEventListener("mousemove", onMove);
  doc.addEventListener("mouseup", onUp);
}

// --- Click / drag gesture on rendered block content -------------------------
//
// OG enters edit mode from block-content-on-mouse-down (GH #368), not from
// mouseup: a held click must show the caret immediately instead of making the
// app feel one click behind. The caret offset is captured at MOUSEDOWN (before
// the previously-edited block's blur reflows the layout), and editing starts at
// once. The document-level drag escalation stays: a drag inside the block
// selects raw text in the editor, and the moment the pointer enters a different
// block the gesture escalates to Tine's block selection (deterministic: purely
// "did the pointer enter a different block", never timing).
const DRAG_THRESHOLD_PX = 4;

interface EditGesture {
  blockId: string;
  offset: number;
  owner: string | null;
  startX: number;
  startY: number;
  escalated: boolean;
  outlineScope: OutlineScope | null;
  /** Caret stops of the mounted textarea, measured lazily on the first in-block
   *  drag move (undefined = not yet measured, null = no layout available). */
  caretPoints: Array<{ x: number; y: number }> | null | undefined;
}

function blockIdAtPoint(doc: Document, x: number, y: number): string | null {
  const el = doc.elementFromPoint(x, y);
  const row = el?.closest?.(".ls-block");
  return row?.getAttribute("data-block-id") ?? null;
}

export function beginEditGesture(
  e: MouseEvent,
  blockId: string,
  offset: number,
  owner: string | null,
  outlineScope: OutlineScope | null,
): void {
  clearSelection(); // a plain gesture replaces any active block selection (shift-click returns before this)
  const g: EditGesture = {
    blockId,
    offset,
    owner,
    startX: e.clientX,
    startY: e.clientY,
    escalated: false,
    outlineScope,
    caretPoints: undefined,
  };
  // The old rendered target is replaced synchronously. Prevent its native
  // focus default from blurring the newly mounted textarea back out, then enter
  // edit before mouseup exactly like OG's mousedown path.
  e.preventDefault();
  startEditing(g.blockId, g.offset, g.owner);
  // The gesture's own window: listeners and hit tests follow the pointer's document.
  const doc = documentOf(e);
  const onMove = (ev: MouseEvent) => {
    const moved =
      Math.abs(ev.clientX - g.startX) > DRAG_THRESHOLD_PX || Math.abs(ev.clientY - g.startY) > DRAG_THRESHOLD_PX;
    if (!moved) return;
    const over = blockIdAtPoint(doc, ev.clientX, ev.clientY);
    if (g.escalated) {
      if (over) extendSelectionTo(over, g.outlineScope);
      return;
    }
    if (over === g.blockId) {
      const active = doc.activeElement;
      if (isElementTag(active, "textarea") && active.classList.contains("block-editor")) {
        if (g.caretPoints === undefined) {
          // Edit entry maps raw source offsets into the actual textarea (code
          // wrappers, for example, are hidden). Continue from that native
          // anchor rather than mixing raw-block and editor coordinates.
          g.offset = active.selectionStart;
          g.caretPoints = textareaCaretPoints(active);
        }
        const points = g.caretPoints;
        if (points?.length) {
          const rect = active.getBoundingClientRect();
          const x = ev.clientX - rect.left + active.scrollLeft;
          const y = ev.clientY - rect.top + active.scrollTop;
          const lineHeight = parseFloat(getComputedStyle(active).lineHeight) || 26;
          let best = 0;
          let bestScore = Infinity;
          for (let i = 0; i < points.length; i++) {
            // Prefer the correct visual row overwhelmingly, then the nearest
            // horizontal caret stop on that row.
            const score = Math.abs(points[i].y + lineHeight / 2 - y) * 10_000 + Math.abs(points[i].x - x);
            if (score < bestScore) {
              best = i;
              bestScore = score;
            }
          }
          active.setSelectionRange(
            Math.min(g.offset, best),
            Math.max(g.offset, best),
            best < g.offset ? "backward" : "forward",
          );
        }
      }
      return;
    }
    if (over && over !== g.blockId) {
      // Crossed into another block: escalate to block selection for the rest of
      // the gesture (never de-escalate — flipping modes mid-drag is jarring).
      g.escalated = true;
      endEdit("select-block");
      doc.getSelection()?.removeAllRanges();
      selectBlock(g.blockId, g.outlineScope);
      extendSelectionTo(over, g.outlineScope);
    }
  };
  const onUp = (ev: MouseEvent) => {
    doc.removeEventListener("mousemove", onMove, true);
    doc.removeEventListener("mouseup", onUp, true);
    if (g.escalated) return; // block selection stands
  };
  doc.addEventListener("mousemove", onMove, true);
  doc.addEventListener("mouseup", onUp, true);
}

/** Click on rendered block content -> raw caret offset for the editor, placing
 *  the caret WHERE you clicked when lsdoc span data can map the rendered leaf
 *  back through source bytes and hidden props. Anything without trustworthy
 *  span data (chips, macro hosts, parser fallback) returns null and the caller
 *  keeps the old end-of-block behaviour. */
export function renderedClickOffset(contentRef: HTMLElement, raw: string, fmt: "md" | "org", e: MouseEvent): number | null {
  const d = document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
  // GH #489/#510: highlight.js has no inline spans. Find the clicked card's
  // parser-owned fence, then map its rendered body to visible-raw coordinates.
  const visible = splitProps(raw, isBuiltinHidden, fmt).visible;
  const range = d.caretRangeFromPoint?.(e.clientX, e.clientY);
  if (range) {
    const cards = Array.from(contentRef.querySelectorAll("pre.code-block > code"));
    const index = cards.findIndex(card => card === range.startContainer || card.contains(range.startContainer));
    const fence = codeFences(visible, fmt).filter(f => f.lang !== "calc")[index];
    const projection = fence && codeBodyProjection(visible, fmt, fence.openEnd);
    const offset = index < 0 ? null : codeCardOffsetFromRange(cards[index].parentElement!, range);
    if (projection && offset !== null) return projection.open.length + Math.min(offset, projection.body.length);
  }
  // GH #465: a click in the empty run-out past the last glyph means "the end",
  // whatever the block ends with. Asked before the span map, because a trailing
  // construct with an invisible closing delimiter (`*italic*`) maps that click
  // to a legitimate-looking interior offset just before the delimiter.
  if (clickBeyondRenderedEnd(contentRef, e.clientX, e.clientY)) return splitProps(raw, isBuiltinHidden, fmt).visible.length;
  if (!range) return null;
  return editorOffsetFromRenderedRange(contentRef, range, raw, isBuiltinHidden, fmt);
}
