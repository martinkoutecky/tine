import { Direction, type EditorView } from "@codemirror/view";

/** Match @codemirror/commands cursorCharLeft/Right: moveByChar takes a direction
 * relative to the line's base direction, and retains assoc/bidiLevel. In view
 * 6.43.15 moveVisually represents outer edges at doc 0/length with an outward
 * association; moveByChar returns the unchanged range only when no move remains.
 * This also respects hard lines and atomic ranges. Interior keys do no layout
 * work here, and the editor handles them normally. */
export function cm6ArrowLeave(key: string, view: EditorView): "backward" | "forward" | null {
  const cursor = view.state.selection.main;
  if (!cursor.empty || (key !== "ArrowLeft" && key !== "ArrowRight")) return null;
  if (cursor.head !== 0 && cursor.head !== view.state.doc.length) return null;
  const ltr = view.textDirectionAt(cursor.head) === Direction.LTR;
  const forward = key === "ArrowRight" ? ltr : !ltr;
  const next = view.moveByChar(cursor, forward);
  return next.eq(cursor, true) ? forward ? "forward" : "backward" : null;
}
