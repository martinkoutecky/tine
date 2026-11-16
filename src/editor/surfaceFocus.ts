import {
  activeSurface, clearFocusSurface, editingId, editingOwner, focusSurfaceFor,
} from "../editorController";
import { documentOf, isElementTag } from "../windowRealm";

/** Arbitrate unscoped editors mounted in multiple occurrences of one block. */
export function focusMountedEditor(
  id: string,
  surface: string,
  editor: () => HTMLTextAreaElement,
  focus: () => void,
): void {
  // The stamp covers siblings in one render flush. The active surface also
  // covers a later mount, e.g. a retained page in a newly opened sidebar.
  // Scoped clicks already name one occurrence and must focus immediately.
  const want = focusSurfaceFor(id)
    ?? (editingOwner() === null ? activeSurface() ?? undefined : undefined);
  if (want === undefined || want === surface) {
    focus();
    // Siblings mounting in this flush must still see the stamp.
    queueMicrotask(() => clearFocusSurface(id));
  } else {
    // Recover if the remembered surface no longer renders the edited block.
    queueMicrotask(() => {
      const ref = editor();
      if (!ref.isConnected || editingId() !== id) return;
      const active = documentOf(ref).activeElement;
      const taken = isElementTag(active, "textarea") && active.classList.contains("block-editor");
      if (!taken) {
        focus();
        clearFocusSurface(id);
      }
    });
  }
}
