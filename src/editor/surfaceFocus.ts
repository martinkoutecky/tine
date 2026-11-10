import { createEffect, on } from "solid-js";
import {
  activeSurface, caretRequest, clearFocusSurface, editingId, editingOwner, focusSurfaceFor, type CaretPos,
} from "../editorController";
import { documentOf } from "../windowRealm";

/** Arbitrate unscoped editors mounted in multiple occurrences of one block, at mount and again whenever
 *  `startEditing` asks for this block while its editor is already mounted. Such a request mounts nothing
 *  (the editing signals do not change), yet it must leave the editor as a fresh mount would: a template
 *  or a one-root paste that takes the place of the empty block being edited keeps its id and its editor,
 *  and the caret still goes to the end of the new text (OG `edit-last-block-after-inserted!`, `:max`).
 *  At mount `focus` takes the request (`takeCaretFor`); later it gets the request's caret and leaves the
 *  request in place, so an editor that mounts for the block afterwards (a move to another journal day
 *  remounts it) still finds it. Call it while the editor's owner is live (onMount); the subscription ends
 *  with it. */
export function focusMountedEditor(
  id: string,
  surface: string,
  editor: () => HTMLElement,
  focus: (again?: { offset: CaretPos }) => void,
): void {
  const arbitrate = (again?: { offset: CaretPos }) => {
    // The stamp covers siblings in one render flush. The active surface also
    // covers a later mount, e.g. a retained page in a newly opened sidebar.
    // Scoped clicks already name one occurrence and must focus immediately.
    const want = focusSurfaceFor(id)
      ?? (editingOwner() === null ? activeSurface() ?? undefined : undefined);
    if (want === undefined || want === surface) {
      focus(again);
      // Siblings mounting in this flush must still see the stamp.
      queueMicrotask(() => clearFocusSurface(id));
    } else {
      // Recover if the remembered surface no longer renders the edited block.
      queueMicrotask(() => {
        const ref = editor();
        if (!ref.isConnected || editingId() !== id) return;
        const active = documentOf(ref).activeElement;
        const taken = active?.matches('textarea.block-editor, [data-cm6-editor], [data-block-editor="pending"]');
        if (!taken) {
          focus(again);
          clearFocusSurface(id);
        }
      });
    }
  };
  arbitrate();
  // The request that mounted this editor is taken above; later ones arrive here. They are handled in a
  // microtask, so an editor the same request unmounts never takes it, and a text resync the gesture
  // queued before it (the textarea's `deleteRange`) cannot move the caret afterwards.
  createEffect(on(caretRequest, (request) => {
    if (request?.id !== id) return;
    queueMicrotask(() => {
      if (caretRequest() !== request || !editor().isConnected || editingId() !== id) return;
      arbitrate(request);
    });
  }, { defer: true }));
}
