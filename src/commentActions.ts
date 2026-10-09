// Creating a margin comment (vision §3.7): Ctrl/Cmd+R in the block editor, or
// the palette's "Comment on selection". A comment is an ordinary child block
// appended after the parent's existing children (thread order = document order),
// written through the document door's own child insert and raw setter as one
// undo step: no new write path.

import { blockWritable, formatForBlock, insertEmptyChildBlock, node as docNode, setRaw, withUndoUnit } from "./document";
import { startEditing } from "./editorController";
import { focusedEditorCommandBridge } from "./editorCommandBridge";
import { graphScopedSignal, refuseStaleWrite } from "./binding";
import { commentRaw, quoteSelectorFor } from "./comments";
import { splitProps, isBuiltinHidden } from "./editor/properties";
import { pushToast } from "./toasts";

/** Append a comment on [start, end) of `editorText` (the parent editor's
 * text) under `parentId` and return its id, or null when the parent cannot be
 * written (read-only page, Guide page, depth cap). The caller has committed
 * `editorText` to the parent already. */
export function createComment(parentId: string, editorText: string, start: number, end: number): string | null {
  const parent = docNode(parentId);
  if (!parent || !blockWritable(parentId)) return null;
  const raw = commentRaw(quoteSelectorFor(editorText, start, end, parent.raw), formatForBlock(parentId));
  let id: string | null = null;
  withUndoUnit("comment:create", [parent.page], () => {
    id = insertEmptyChildBlock(parentId, parent.children.length);
    if (id) setRaw(id, raw, { timetracking: false });
  });
  return id;
}

/** Create the comment and put the caret on its empty body line. */
export function commentAndEdit(parentId: string, editorText: string, start: number, end: number, surface: string | null = null): string | null {
  const id = createComment(parentId, editorText, start, end);
  if (id) startEditing(id, 0, null, surface);
  return id;
}

// The palette closes the editor before its command runs (opening it blurs the
// textarea), so the selection is taken when the palette opens. Graph-scoped: a
// graph switch clears it, and the command refuses a block whose text changed.
interface PaletteSelection { blockId: string; text: string; start: number; end: number }
const [paletteSelection, setPaletteSelection] = graphScopedSignal<PaletteSelection>();
// Whether a selection was remembered at all (names no graph content), so a
// selection that a graph switch cleared is refused rather than reported missing.
let selectionRemembered = false;

/** Called as the switcher or command palette opens. */
export function rememberEditorSelectionForPalette(): void {
  const bridge = focusedEditorCommandBridge();
  const selection = bridge?.textSelection?.();
  selectionRemembered = !!(bridge && selection);
  setPaletteSelection(bridge && selection ? { blockId: bridge.blockId, ...selection } : null);
}

/** "Comment on selection" from the palette. */
export function commentOnPaletteSelection(): void {
  const remembered = selectionRemembered;
  selectionRemembered = false;
  // Refusal scenario (I-20): the graph was switched while the palette was open,
  // so the remembered block belongs to a graph that is no longer bound.
  if (remembered && paletteSelection() === null) return refuseStaleWrite("The comment");
  const held = paletteSelection();
  setPaletteSelection(null);
  if (!held) {
    pushToast("Select text in a block first, then choose Comment on selection (or press Ctrl+R while editing)", "warn");
    return;
  }
  const parent = docNode(held.blockId);
  const format = formatForBlock(held.blockId);
  // Refusal scenario: the block's text changed while the palette was open (an
  // external editor or a synced copy landed), so the remembered offsets no
  // longer name the selected words. Nothing is written; selecting again works.
  if (!parent || splitProps(parent.raw, isBuiltinHidden, format).visible !== held.text) {
    pushToast("The block changed after you selected text; select it again to comment", "warn");
    return;
  }
  commentAndEdit(held.blockId, held.text, held.start, held.end);
}
