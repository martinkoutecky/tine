// Showing and hiding the comment margin (GH #845). The preference itself is a
// device setting kept in ui.ts like wide and document mode; this module is how
// every caller flips it, because hiding must not pull the thread out from under
// a caret or an unsent reply, and a comment must never be written into something
// invisible. Nothing here touches the graph.

import { editingId, startEditing } from "./editorController";
import { formatForBlock, node as docNode } from "./document";
import { commentMarginHidden, forceCommentMarginShown, persistCommentMarginHidden } from "./commentMarginPref";
import { clearOnBindingInvalidated } from "./binding";
import { threadRootOf } from "./margin";
import { pushToast } from "./toasts";
import { msg } from "./i18n";

// Unsent reply drafts live in the composers' textareas, outside the document.
const replyDrafts = new Map<() => string, string | undefined>();
clearOnBindingInvalidated(() => replyDrafts.clear());
/** A mounted reply composer registers how to read its draft. */
export function registerReplyDraft(draft: () => string, root?: string): () => void {
  replyDrafts.set(draft, root);
  return () => void replyDrafts.delete(draft);
}

/** Visibility changes must not discard a mounted composer's unsent text. */
export function hasPendingReplyDraft(roots?: readonly string[]): boolean {
  return [...replyDrafts].some(([draft, root]) => (!roots || root !== undefined && roots.includes(root)) && !!draft().trim());
}

/** Move a caret that sits in a comment thread to the commented block (its text is
 * already in the document: the editor commits as it is typed). False when the
 * editor refused to let go, in which case nothing may be hidden. */
function leaveThread(): boolean {
  const id = editingId();
  if (!id || !docNode(id)) return true;
  const root = threadRootOf(id, formatForBlock(id));
  if (root === null) return true;
  const parent = docNode(root)?.parent ?? null;
  if (parent === null || !docNode(parent)) return true;
  startEditing(parent, docNode(parent).raw.length);
  return editingId() !== id;
}

/** Hide (`visible` false) or show the margin. Hiding is refused, with a toast,
 * while a reply draft holds text, and waits for a caret in a thread to move out.
 * Returns whether the margin now has the asked visibility. */
export function setCommentMarginVisible(visible: boolean): boolean {
  if (visible === !commentMarginHidden()) return true;
  if (visible) return persistCommentMarginHidden(false);
  if (hasPendingReplyDraft()) {
    pushToast(msg("comment-margin-draft-pending"), "warn");
    return false;
  }
  if (!leaveThread()) return false;
  return persistCommentMarginHidden(true);
}

/** The palette command, the shortcut and the Settings toggle. */
export function toggleCommentMargin(): void {
  setCommentMarginVisible(commentMarginHidden());
}

/** A comment was just written, or a link led to one: show the margin so it is
 * seen, whether or not storage keeps the preference. */
export function revealCommentMargin(): void {
  if (commentMarginHidden()) forceCommentMarginShown();
}

/** Open a block a link or search result points at: a comment, or a reply in a
 * thread, shows the margin when it is hidden. */
export function revealCommentMarginFor(blockId: string): void {
  if (!commentMarginHidden() || !docNode(blockId)) return;
  if (threadRootOf(blockId, formatForBlock(blockId)) !== null) forceCommentMarginShown();
}
