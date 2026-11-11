// Creating a margin comment (vision §3.7): Ctrl/Cmd+R in the block editor, or
// the palette's "Comment on selection". A comment is an ordinary child block
// appended after the parent's existing children (thread order = document order),
// written through the document door's own child insert and raw setter as one
// undo step: no new write path.

import { batch } from "solid-js";
import { blockWritable, formatForBlock, insertEmptyChildBlock, node as docNode, setRaw, setBlockProperty, setCollapsedBlocks, withUndoUnit } from "./document";
import { editingId, endEdit, startEditing } from "./editorController";
import { focusedEditorCommandBridge } from "./editorCommandBridge";
import { graphScopedSignal, refuseStaleWrite } from "./binding";
import { anchorQuote, commentRaw, replyRaw, normalizeQuoteText, normalizeWithMap, occurrences, quoteSelectorFor, quoteSelectorOf, stampCommentDate, COMMENT_RESOLVED_KEY } from "./comments";
import { facetsOf } from "./render/facets";
import { splitProps, BUILTIN_HIDE } from "./editor/properties";
import { pushToast } from "./toasts";
import { msg, t } from "./i18n";
import { revealCommentMargin, hasPendingReplyDraft } from "./commentMargin";
import { commentAuthorName } from "./commentIdentity";
import { runWithCommentName } from "./commentNamePrompt";
import { selectionOf } from "./windowRealm";

/** Fold the passage's thread roots in one existing document transaction. */
export function collapseComments(id: string): void {
  const parent = docNode(id);
  if (parent) setCollapsedBlocks(parent.children.filter(child => threadRootOf(child) === child), true);
}
import { threadRootOf } from "./margin";
import { deleteBlock } from "./document";
import { commentGroups, passageKey } from "./commentGroups";
import { focusCommentReply } from "./commentReplyFocus";
import { threadResolved } from "./commentResolution";

/** Resolution belongs to the outermost root and the ordinary property door. */
export function setThreadResolved(commentId: string, resolved: boolean): void {
  const root = threadRootOf(commentId);
  if (!root || !blockWritable(root) || threadResolved(root) === resolved) return;
  if (resolved && hasPendingReplyDraft([root])) { pushToast(msg("comment-margin-draft-pending"), "warn"); return; }
  endEdit("page-navigation");
  if (editingId() !== null) return;
  batch(() => withUndoUnit("comment:resolve", [docNode(root)!.page], () => {
    setBlockProperty(root, COMMENT_RESOLVED_KEY, resolved ? "true" : null);
  }));
}

/** Append a comment on [start, end) of `editorText` (the parent editor's
 * text) under `parentId` and return its id, or null when the parent cannot be
 * written (read-only page, Guide page, depth cap). The caller has committed
 * `editorText` to the parent already. */
export function createComment(parentId: string, editorText: string, start: number, end: number): string | null {
  const parent = docNode(parentId);
  if (!parent || !blockWritable(parentId)) return null;
  const raw = commentRaw(quoteSelectorFor(editorText, start, end, parent.raw, formatForBlock(parentId)), formatForBlock(parentId), commentAuthorName());
  const root = threadRootOf(parentId, formatForBlock(parentId));
  const destination = docNode(root ?? parentId)!;
  let id: string | null = null;
  // Publish structure and quote metadata together. Inline ordering deliberately
  // reads text untracked on structural changes; exposing the empty child first
  // made it mount after the subtree and sent the caret there (GH #110).
  batch(() => {
    withUndoUnit("comment:create", [parent.page], () => {
      id = insertEmptyChildBlock(destination.id, destination.children.length);
      if (id) {
        setRaw(id, stampCommentDate(raw, formatForBlock(parentId)), { timetracking: false });
        if (root && threadResolved(root)) setBlockProperty(root, COMMENT_RESOLVED_KEY, null);
      }
    });
    if (id) {
      // GH #845: a comment is never written into a hidden margin.
      revealCommentMargin();
    }
  });
  return id;
}

/** Create the comment and put the caret on its empty body line. */
export function commentAndEdit(parentId: string, editorText: string, start: number, end: number, surface: string | null = null, owner: string | null = null): string | null {
  return batch(() => {
    const parent = docNode(parentId);
    if (parent && blockWritable(parentId) && threadRootOf(parentId) === null) {
      const format = formatForBlock(parentId);
      const key = passageKey(parent.raw, quoteSelectorFor(editorText, start, end, parent.raw, format), format);
      const group = key === null ? undefined : commentGroups(parentId, format, true).find(ids => {
        const selector = quoteSelectorOf(facetsOf(docNode(ids[0])!.raw, format).properties)!;
        return passageKey(parent.raw, selector, format) === key;
      });
      if (group) {
        endEdit("page-navigation");
        revealCommentMargin();
        focusCommentReply(group.at(-1)!, surface, false, true);
        return group.at(-1)!;
      }
    }
    const id = createComment(parentId, editorText, start, end);
    if (id) startEditing(id, 0, owner, surface);
    return id;
  });
}

/** Ctrl/Cmd+R in the block editor `ref`: commit its text, then comment on the
 * selection (the whole block when nothing is selected). */
export function commentFromEditor(id: string, ref: Pick<HTMLTextAreaElement, "value" | "selectionStart" | "selectionEnd">, commit: (text: string) => void, surface: string | null, owner: string | null = null): void {
  const { value, selectionStart, selectionEnd } = ref;
  commit(value);
  const rawBefore = docNode(id)?.raw;
  // The first comment of a person with no name asks for one first (tine#841);
  // the block may have changed while the dialog was open.
  runWithCommentName(() => {
    if (docNode(id)?.raw !== rawBefore) return pushToast(msg("comment-block-changed"), "warn");
    if (!commentAndEdit(id, value, selectionStart, selectionEnd, surface, owner)) pushToast(msg("comment-block-cannot-take"), "warn");
  });
}

/** Explicit New thread preserves the group's passage and appends a separate
 * root through the same document door as every comment creation. */
export function appendThreadForPassage(commentId: string, surface: string | null = null): string | null {
  const comment = docNode(commentId), format = formatForBlock(commentId);
  const selector = comment && quoteSelectorOf(facetsOf(comment.raw, format).properties);
  const parent = comment?.parent ? docNode(comment.parent) : null;
  if (!parent || !selector || !blockWritable(parent.id)) return null;
  const anchor = anchorQuote(parent.raw, selector, format);
  if (anchor.kind !== "range" && anchor.kind !== "whole") return null;
  // New roots obey the same context-to-uniqueness writer as editor selections,
  // including when the source thread is legacy text without context properties.
  const freshSelector = anchor.kind === "range"
    ? quoteSelectorFor(parent.raw, anchor.start, anchor.end, parent.raw, format) : selector;
  const parentRaw = parent.raw;
  const sourceRaw = comment.raw;
  let id: string | null = null;
  runWithCommentName(() => batch(() => {
    if (docNode(parent.id) !== parent || parent.raw !== parentRaw || docNode(commentId) !== comment || comment.raw !== sourceRaw) return pushToast(msg("comment-block-changed"), "warn");
    withUndoUnit("comment:create", [parent.page], () => {
      id = insertEmptyChildBlock(parent.id, parent.children.length);
      if (id) setRaw(id, stampCommentDate(commentRaw(freshSelector, format, commentAuthorName()), format), { timetracking: false });
    });
    if (id) { revealCommentMargin(); startEditing(id, 0, null, surface); }
  }));
  return id;
}

/** A composer draft is local UI state until it has content. Commit the child
 * and its body together through the ordinary document door, in one undo unit. */
export function submitReply(commentId: string, text: string): string | null {
  const comment = docNode(commentId);
  if (!text.trim() || !comment || !blockWritable(commentId)
    || threadRootOf(commentId, formatForBlock(commentId)) === null) return null;
  const root = docNode(threadRootOf(commentId, formatForBlock(commentId))!)!;
  let id: string | null = null;
  batch(() => withUndoUnit("comment:reply", [comment.page], () => {
    id = insertEmptyChildBlock(root.id, root.children.length);
    if (id) {
      const author = commentAuthorName();
      const metadata = author ? replyRaw(formatForBlock(commentId), author) : "";
      setRaw(id, text + stampCommentDate(metadata || "\n", formatForBlock(commentId)), { timetracking: false });
      if (threadResolved(root.id)) setBlockProperty(root.id, COMMENT_RESOLVED_KEY, null);
    }
  }));
  return id;
}

/** Ctrl/Cmd+R in a nonempty reply draft follows the same quoted-child action
 * as the block editor. Persist the draft and quoted child as one undo unit. */
export function commentFromReplyDraft(commentId: string, text: string, start: number, end: number, surface: string | null): string | null {
  const comment = docNode(commentId);
  if (!comment || !text.trim()) return null;
  let reply: string | null = null;
  withUndoUnit("comment:reply", [comment.page], () => {
    reply = submitReply(commentId, text);
    if (reply && !commentAndEdit(reply, text, start, end, surface)) startEditing(reply, start, null, surface);
  });
  return reply;
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
  // Refusal scenario: the block's text changed while the palette was open (an
  // external editor or a synced copy landed), so the remembered offsets no
  // longer name the selected words. Nothing is written; selecting again works.
  commentOnRenderedSelection(held);
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
  // The check runs after the name dialog (tine#841), which may stay open a while.
  runWithCommentName(() => {
    const parent = docNode(held.blockId);
    if (!parent || splitProps(parent.raw, BUILTIN_HIDE, formatForBlock(held.blockId)).visible !== held.text) {
      pushToast(msg("comment-block-changed"), "warn");
      return;
    }
    if (!commentAndEdit(held.blockId, held.text, held.start, held.end)) pushToast(msg("comment-block-cannot-take"), "warn");
  });
}
