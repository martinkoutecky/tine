import { nextVisibleOrExtend } from "../document";
import { captureEditIntent, startEditing } from "../editorController";
import { documentOf } from "../windowRealm";

/** A feed load may outlive the editor gesture that requested it. Keep its
 * completion bound to that exact mounted occurrence and edit intent. */
export async function navigateJournalDown(request: {
  id: string;
  col: number;
  surface: string | null;
  editor: HTMLTextAreaElement;
  current: (editor: HTMLTextAreaElement) => boolean;
}): Promise<void> {
  const { id, col, surface, editor, current } = request;
  const intentIsCurrent = captureEditIntent();
  const next = await nextVisibleOrExtend(id);
  if (next && intentIsCurrent() && current(editor) && editor.isConnected
    && documentOf(editor).activeElement === editor) {
    startEditing(next, { col, edge: "first" }, null, surface);
  }
}
