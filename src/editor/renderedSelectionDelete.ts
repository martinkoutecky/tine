// Delete/Backspace pressed while a NON-EDITOR text selection exists in a
// rendered (not-editing) block — OG contenteditable parity: in OG the rendered
// view IS the editor, so deleting a rendered selection deletes the text. In
// Tine the rendered view and the textarea are separate, so the same gesture's
// keypress previously reached no editor and died silently (selection highlight
// stayed, nothing changed — Martin's "quick select + delete does nothing").
//
// Conservative contract (data safety first): only act on a selection that lies
// ENTIRELY within ONE block's rendered content (multi-block rendered selections
// merge blocks in OG — too destructive to guess, so they stay a no-op), only
// when the block is writable, not an annotation/calc block, not being edited
// (the textarea owns keys then), and only when the rendered points map
// unambiguously back to source offsets. On success the text is deleted through
// the normal store path (undo snapshot, dirty marking — same as an editor
// delete), and the block opens in the editor with the caret at the join point
// so the user lands exactly where OG would leave them.

import { node as docNode, setRaw, blockWritable, pageByName } from "../document";
import { startEditing, editingId } from "../editorController";
import { splitProps, joinProps, BUILTIN_HIDE } from "../editor/properties";
import { rangeEndDependsOnGeneratedOutput, sourceRangeFromRenderedSelection } from "../render/spans";
import { isAnnotationBlock } from "./annotation";
import { calcSource } from "./calc";
import { queryAllWindows, selectionOf } from "../windowRealm";

function blockRowOf(node: Node | null): Element | null {
  if (!node) return null;
  const el = node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
  return el?.closest?.(".ls-block") ?? null;
}

/** Delete the current rendered text selection inside one block. Returns true
 *  when the key press was consumed (text deleted through `setRaw`: one undo
 *  step, one `save-block` dirty mark, then the block opens in the editor at the
 *  join point); false to leave the event to its default handling. Refuses
 *  (false) a collapsed or cross-block selection, a reference/embed/query
 *  rendering, a selection end that depends on generated output (a query's, a
 *  failed-region or embed-depth notice), a selection end inside a construct
 *  with no exact text mapping that the selection does not cover whole, the
 *  block being edited, a read-only page, annotation or calc blocks, an open
 *  modal, and any selection point that does not map to source.
 *  Cost: O(length of that block). */
export function deleteRenderedTextSelection(): boolean {
  if (typeof window === "undefined" || typeof document === "undefined") return false;
  // A modal owns keystrokes while open — never mutate background blocks behind it.
  if (queryAllWindows(".modal-overlay")) return false;
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
  // A query's output and the app's notices are generated: an end there names
  // none of this block's text, and inside a heading, quote or callout it would
  // snap to an edge of that construct and take the macro's source with it. So
  // would an end in text of that construct that has no source position of its
  // own (a callout's title): its edge is chosen by a length that counts the
  // generated text. Checked after the refusals above, so a refused key
  // searches nothing.
  if (rangeEndDependsOnGeneratedOutput(wrapper, range.startContainer)
    || rangeEndDependsOnGeneratedOutput(wrapper, range.endContainer)) return false;
  // An end inside a construct with no exact text mapping (inline HTML, a page
  // reference, a tag, math) names source only when the selection covers that
  // whole construct; otherwise the splice would take text or media that was
  // not selected. Then the selection stays as it is and nothing is written.
  const selected = sourceRangeFromRenderedSelection(wrapper, range, node.raw, BUILTIN_HIDE, fmt);
  if (!selected) return false;
  const { start, end } = selected;

  const split = splitProps(node.raw, BUILTIN_HIDE, fmt);
  const s = Math.min(start, split.visible.length);
  const e = Math.min(end, split.visible.length);
  if (s === e) return false;
  const newVisible = split.visible.slice(0, s) + split.visible.slice(e);
  const newRaw = joinProps(newVisible, split.hidden, fmt, node.raw);
  if (newRaw === node.raw) return false;

  setRaw(id, newRaw);
  // The rendered selection now points at stale text — clear it, and land in the
  // editor at the join point so the next keystroke continues editing there.
  sel.removeAllRanges();
  startEditing(id, s);
  return true;
}
