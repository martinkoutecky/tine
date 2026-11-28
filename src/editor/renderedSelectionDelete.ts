// Delete/Backspace pressed while a NON-EDITOR text selection exists in a
// rendered (not-editing) block. In OG the rendered view IS the editor; in Tine
// the rendered view and the editor are separate, so the key reached no editor
// and died silently. The key never writes the block from here:
// mapping a rendered range back to source bytes is not exact (inline HTML, link
// destinations, media, hidden markup), and every splice from it lost unselected
// text. Instead it opens the block's editor exactly as a click and shift-click
// at the two ends would, with that range selected, so the user sees the source
// that the next Delete removes.
//
// Gates (unchanged): only a selection that lies ENTIRELY within ONE block's
// rendered content, in the live outline, of a writable block that is not an
// annotation or calc block and not being edited, whose two ends map to source.

import { node as docNode, blockWritable, pageByName } from "../document";
import { startEditing, editingId } from "../editorController";
import { splitProps, BUILTIN_HIDE } from "../editor/properties";
import { FOREIGN_SOURCE_ATTR, editorOffsetFromRenderedRange, rangeEndDependsOnGeneratedOutput } from "../render/spans";
import { codeCardBodyAt } from "../components/blockGestures";
import { isAnnotationBlock } from "./annotation";
import { calcSource } from "./calc";
import { selectionOf } from "../windowRealm";

// The rendered instance (Block's content wrapper) -> the owner its click passes
// to `startEditing`, so the handoff opens the editor in the selecting instance.
const instanceOwners = new WeakMap<Element, string>();
export function noteInstanceOwner(wrapper: Element, owner: string): void {
  instanceOwners.set(wrapper, owner);
}

// The key that handed a rendered selection to its editor, while it is held:
// its auto-repeat must not reach the editor that just opened and delete the
// selection the user has not seen yet. Set only by a consumed handoff; cleared
// by that key's keyup, a window blur, or a keydown of any other key.
let heldHandoffKey: string | null = null;

/** The global keydown handler's first step: true when `e` is the held handoff
 *  key's auto-repeat, which is then consumed and stopped before any editor. O(1). */
export function swallowHeldHandoffRepeat(e: KeyboardEvent): boolean {
  if (heldHandoffKey === null) return false;
  if (e.key === heldHandoffKey && e.repeat) {
    e.preventDefault();
    e.stopImmediatePropagation();
    return true;
  }
  heldHandoffKey = null;
  return false;
}

/** Window keyup (capture) and blur listener ending the hold. O(1). */
export function releaseHeldHandoffKey(e: Event): void {
  if (e.type !== "keyup" || (e as KeyboardEvent).key === heldHandoffKey) heldHandoffKey = null;
}

function blockRowOf(node: Node | null): Element | null {
  if (!node) return null;
  const el = node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
  return el?.closest?.(".ls-block") ?? null;
}

/** Whether `node` lies below a {@link FOREIGN_SOURCE_ATTR} element inside `wrapper`. O(depth of `node`). */
function inForeignSource(wrapper: Element, node: Node): boolean {
  const el = node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
  const marker = el?.closest(`[${FOREIGN_SOURCE_ATTR}]`);
  return !!marker && wrapper.contains(marker);
}

/** Hand the current rendered text selection inside one block to its editor.
 *  Returns true when the key press was consumed: `startEditing` was asked to
 *  open the block in the selecting instance with the mapped range selected (a
 *  body range of one of the block's own code cards in body mode), as a click entry would; nothing is
 *  written by this function. The rendered selection is cleared once the block
 *  is editing; when an entry guard refuses, this function leaves it (the guard
 *  may move focus and the selection to what it refuses for). Returns false (the key
 *  goes on) for a collapsed or cross-block selection, a reference/embed/query
 *  rendering, a selection end inside another source drawn in the block (an
 *  embed, a block reference's target, a macro's expansion), a selection end that depends on generated output (a query's, a
 *  failed-region or embed-depth notice), the block being edited, a read-only
 *  page, annotation or calc blocks, a selection point that does
 *  not map to source, and a range that maps to nothing. On true, `key` is held
 *  (see `swallowHeldHandoffRepeat`) until released.
 *  Cost: O(length of that block). */
export function openRenderedSelectionInEditor(key: string): boolean {
  if (typeof window === "undefined" || typeof document === "undefined") return false;
  const sel = selectionOf();
  if (!sel || sel.rangeCount === 0) return false;
  const range = sel.getRangeAt(0);
  if (range.collapsed) return false;

  const row = blockRowOf(range.startContainer);
  if (!row || row !== blockRowOf(range.endContainer)) return false;
  // Linked/unlinked references, embeds and query results (RefBlocks) render the
  // SAME `.ls-block > .block-main > .block-content-wrapper` shape and carry the
  // source block's id, but they are a different renderer over a query result and
  // are deliberately not edited in place. Mapping their DOM onto the live
  // `doc.byId` raw would splice bytes computed from a tree that can legitimately
  // differ from it. Only the live outline owns this gesture.
  if (row.classList.contains("ref-block")) return false;
  const wrapper = row.querySelector(":scope > .block-main .block-content-wrapper");
  if (!wrapper || !wrapper.contains(range.startContainer) || !wrapper.contains(range.endContainer)) {
    return false;
  }
  const id = row.getAttribute("data-block-id");
  if (!id) return false;
  if (editingId() === id) return false; // that block's editor owns the keys
  const node = docNode(id);
  if (!node || !blockWritable(id)) return false;
  // Annotation and calc blocks have non-plain rendered views; never splice them
  // through reconstructed text state (same rule as editor merge/delete paths).
  const page = pageByName(node.page);
  if (!page) return false;
  const fmt = page.format === "org" ? "org" : "md";
  if (isAnnotationBlock(node.raw, fmt) || calcSource(node.raw) !== null) return false;
  // A point inside another source drawn in the block (an embed, a block
  // reference's target, a query's results, a macro's expansion, a notice, and
  // every code card they draw) names none of this block's text: the key does
  // nothing, before any mapping.
  if (inForeignSource(wrapper, range.startContainer) || inForeignSource(wrapper, range.endContainer)) return false;
  // A query's output and the app's notices are generated: an end there names
  // none of this block's text, and inside a heading, quote or callout it would
  // snap to an edge of that construct and take the macro's source with it. So
  // would an end in text of that construct that has no source position of its
  // own (a callout's title): its edge is chosen by a length that counts the
  // generated text. Checked after the refusals above, so a refused key
  // searches nothing.
  if (rangeEndDependsOnGeneratedOutput(wrapper, range.startContainer)
    || rangeEndDependsOnGeneratedOutput(wrapper, range.endContainer)) return false;

  const split = splitProps(node.raw, BUILTIN_HIDE, fmt);
  const backward = !!sel.anchorNode && sel.anchorNode === range.endContainer && sel.anchorOffset === range.endOffset;
  const direction = backward ? "backward" as const : "forward" as const;
  // Inside one of the block's own code cards each end is mapped as a body click
  // maps it, and the editor opens in body mode at that body, in body coordinates.
  const content = wrapper.querySelector(".block-content") ?? wrapper;
  const startPoint = { startContainer: range.startContainer, startOffset: range.startOffset };
  const endPoint = { startContainer: range.endContainer, startOffset: range.endOffset };
  const card = codeCardBodyAt(content, split.visible, fmt, range.startContainer);
  let selection: { start: number; end: number; direction: "forward" | "backward"; codeBodyStart?: number };
  if (card && card.card.contains(range.endContainer)) {
    const start = card.offset(startPoint), end = card.offset(endPoint);
    if (start >= end) return false;
    selection = { start, end, direction, codeBodyStart: card.bodyStart };
  } else {
    const start = editorOffsetFromRenderedRange(wrapper, startPoint, node.raw, BUILTIN_HIDE, fmt);
    const end = editorOffsetFromRenderedRange(wrapper, endPoint, node.raw, BUILTIN_HIDE, fmt);
    if (start === null || end === null || end < start) return false;
    const s = Math.min(start, split.visible.length);
    const e = Math.min(end, split.visible.length);
    if (s === e) return false;
    selection = { start: s, end: e, direction };
  }

  heldHandoffKey = key;
  startEditing(id, selection, instanceOwners.get(wrapper) ?? null);
  // The editor now shows the range; on a refused entry the selection is the guard's to keep or move.
  // Clear only a document selection that is still the rendered range: in WebKit
  // a focused textarea's own selection IS the document selection, and clearing
  // that would drop the range the editor was just given.
  const now = sel.rangeCount ? sel.getRangeAt(0) : null;
  if (editingId() === id && now && now.startContainer === range.startContainer && now.startOffset === range.startOffset
    && now.endContainer === range.endContainer && now.endOffset === range.endOffset) sel.removeAllRanges();
  return true;
}
