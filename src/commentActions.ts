// Creating a margin comment (vision §3.7): Ctrl/Cmd+R in the block editor, or
// the palette's "Comment on selection". A comment is an ordinary child block
// appended after the parent's existing children (thread order = document order),
// written through the document door's own child insert and raw setter as one
// undo step: no new write path.

import { blockWritable, formatForBlock, insertEmptyChildBlock, node as docNode, setRaw, withUndoUnit } from "./document";
import { editingId, endEdit, startEditing } from "./editorController";
import { focusedEditorCommandBridge } from "./editorCommandBridge";
import { graphScopedSignal, refuseStaleWrite } from "./binding";
import { commentRaw, replyRaw, normalizeQuoteText, normalizeWithMap, occurrences, quoteSelectorFor, quoteSelectorOf } from "./comments";
import { blockRegions } from "./render/parse";
import { facetsOf } from "./render/facets";
import { utf8ByteLength } from "./render/spans";
import { splitProps, BUILTIN_HIDE } from "./editor/properties";
import { pushToast } from "./toasts";
import { msg, t } from "./i18n";
import { commentsWritten } from "./margin";
import { commentAuthorName } from "./commentIdentity";
import { selectionOf } from "./windowRealm";
import { threadRootOf } from "./margin";
import { deleteBlock } from "./document";

/** Append a comment on [start, end) of `editorText` (the parent editor's
 * text) under `parentId` and return its id, or null when the parent cannot be
 * written (read-only page, Guide page, depth cap). The caller has committed
 * `editorText` to the parent already. */
export function createComment(parentId: string, editorText: string, start: number, end: number): string | null {
  const parent = docNode(parentId);
  if (!parent || !blockWritable(parentId)) return null;
  const raw = commentRaw(quoteSelectorFor(editorText, start, end, parent.raw), formatForBlock(parentId), commentAuthorName());
  let id: string | null = null;
  withUndoUnit("comment:create", [parent.page], () => {
    id = insertEmptyChildBlock(parentId, parent.children.length);
    if (id) setRaw(id, raw, { timetracking: false });
  });
  if (id) commentsWritten();
  return id;
}

/** Create the comment and put the caret on its empty body line. */
export function commentAndEdit(parentId: string, editorText: string, start: number, end: number, surface: string | null = null): string | null {
  const id = createComment(parentId, editorText, start, end);
  if (id) startEditing(id, 0, null, surface);
  return id;
}

/** Ctrl/Cmd+R in the block editor `ref`: commit its text, then comment on the
 * selection (the whole block when nothing is selected). */
export function commentFromEditor(id: string, ref: HTMLTextAreaElement, commit: (text: string) => void, surface: string | null): void {
  const { value, selectionStart, selectionEnd } = ref;
  commit(value);
  if (!commentAndEdit(id, value, selectionStart, selectionEnd, surface)) pushToast(msg("comment-block-cannot-take"), "warn");
}

/** Reply to this exact comment/reply, as an ordinary last child in its thread.
 * No quote is needed for the visible Reply action. */
export function replyAndEdit(commentId: string, surface: string | null = null): string | null {
  const comment = docNode(commentId);
  if (!comment || !blockWritable(commentId) || threadRootOf(commentId, formatForBlock(commentId)) === null) return null;
  let id: string | null = null;
  withUndoUnit("comment:reply", [comment.page], () => {
    id = insertEmptyChildBlock(commentId, comment.children.length);
    const author = commentAuthorName();
    if (id && author) setRaw(id, replyRaw(formatForBlock(commentId), author), { timetracking: false });
  });
  if (id) startEditing(id, 0, null, surface);
  return id;
}

/** The document door removes the complete subtree and owns its single undo
 * step. End an editor within that subtree before its nodes disappear. */
export function deleteComment(commentId: string): void {
  if (!blockWritable(commentId)) return;
  if (threadRootOf(commentId, formatForBlock(commentId)) === null) return;
  for (let id = editingId(); id !== null; id = docNode(id)?.parent ?? null) {
    if (id === commentId) { endEdit("delete-block"); break; }
  }
  deleteBlock(commentId);
}

/** Enter in a comment's text, before its property lines, starts a reply: a new
 * last child (thread order = document order), and the caret moves there. A split
 * would carry the `quote::` lines into the new block and turn the comment into
 * an ordinary block. Returns false when Enter should behave as everywhere else
 * (an ordinary block, or the caret among the property lines). The caller has
 * committed `editorText`. One parse of the editor text, only on Enter in a
 * comment (the comment test itself is a cached facet lookup). */
export function replyOnEnter(commentId: string, editorText: string, caret: number, surface: string | null = null): boolean {
  const comment = docNode(commentId);
  if (!comment || comment.parent === null) return false;
  const format = formatForBlock(commentId);
  if (quoteSelectorOf(facetsOf(comment.raw, format).properties) === null) return false;
  const lines = blockRegions(editorText, format).properties.map((p) => p.line[0]);
  if (lines.length > 0 && utf8ByteLength(editorText.slice(0, caret)) > Math.min(...lines)) return false;
  return replyAndEdit(commentId, surface) !== null;
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
  if (remembered && paletteSelection() === null) return refuseStaleWrite(t("comment-stale-write-what"));
  const held = paletteSelection();
  setPaletteSelection(null);
  if (!held) {
    pushToast(msg("comment-select-first"), "warn");
    return;
  }
  const parent = docNode(held.blockId);
  const format = formatForBlock(held.blockId);
  // Refusal scenario: the block's text changed while the palette was open (an
  // external editor or a synced copy landed), so the remembered offsets no
  // longer name the selected words. Nothing is written; selecting again works.
  if (!parent || splitProps(parent.raw, BUILTIN_HIDE, format).visible !== held.text) {
    pushToast(msg("comment-block-changed"), "warn");
    return;
  }
  commentAndEdit(held.blockId, held.text, held.start, held.end);
}

// --- the right-click route: text selected in a block's rendered view --------
// The block menu (right-click on rendered text) offers "Comment on selection"
// when words of that block are selected. The editor keeps its native text menu
// (cut, copy, paste, spelling), so there Ctrl/Cmd+R and the palette are the routes.

/** The words selected inside `blockId`'s rendered text, as an editor-text
 * selection [start, end) of the block's visible text, or null when nothing in
 * this block is selected or the words cannot be found in its text (formatting
 * inside the selection). The occurrence is the same ordinal the rendered text
 * shows, so a repeated phrase anchors where it was selected. */
export function renderedSelectionIn(blockId: string): PaletteSelection | null {
  const node = docNode(blockId);
  const selection = selectionOf();
  if (!node || !selection || selection.isCollapsed || selection.rangeCount === 0) return null;
  const range = selection.getRangeAt(0);
  const common = range.commonAncestorContainer;
  const element = common.nodeType === 1 ? (common as Element) : common.parentElement;
  const row = element?.closest("[data-block-id]");
  if (!row || row.getAttribute("data-block-id") !== blockId || !element?.closest(".block-content")) return null;
  const words = selection.toString();
  const needle = normalizeQuoteText(words);
  if (!needle) return null;
  const visible = splitProps(node.raw, BUILTIN_HIDE, formatForBlock(blockId)).visible;
  const text = normalizeWithMap(visible);
  // Which occurrence: those that precede the selection in the rendered text.
  const content = element.closest(".block-content")!;
  const before = content.ownerDocument.createRange();
  before.selectNodeContents(content);
  before.setEnd(range.startContainer, range.startOffset);
  const ordinal = occurrences(normalizeQuoteText(before.toString()), needle).length;
  const found = occurrences(text.text, needle);
  if (found.length === 0) return null;
  const at = found[Math.min(ordinal, found.length - 1)];
  return { blockId, text: visible, start: text.starts[at], end: text.ends[at + needle.length - 1] };
}

/** Run "Comment on selection" for a selection captured by {@link renderedSelectionIn}.
 * Refuses (a toast, nothing written) when the block's text changed since, the
 * same stale-selection rule as the palette. */
export function commentOnRenderedSelection(held: NonNullable<ReturnType<typeof renderedSelectionIn>>): void {
  const parent = docNode(held.blockId);
  if (!parent || splitProps(parent.raw, BUILTIN_HIDE, formatForBlock(held.blockId)).visible !== held.text) {
    pushToast(msg("comment-block-changed"), "warn");
    return;
  }
  if (!commentAndEdit(held.blockId, held.text, held.start, held.end)) pushToast(msg("comment-block-cannot-take"), "warn");
}
