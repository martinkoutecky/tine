// WebKitGTK paints no native caret in an empty contenteditable line (a lone <br>) at some viewport positions: a new
// empty block opened in CM6 showed no caret until the first character was typed. The caret of an EMPTY document is
// therefore drawn by CM itself (a cursor layer); once there is text the native caret takes over, unchanged.
import { EditorSelection, type Extension } from "@codemirror/state";
import { EditorView, RectangleMarker, layer } from "@codemirror/view";

export const EMPTY_DOC_CLASS = "cm6-empty-doc";

export function cm6EmptyCaret(): Extension {
  return [
    EditorView.editorAttributes.compute(["doc"], state => ({ class: state.doc.length === 0 ? EMPTY_DOC_CLASS : "" })),
    layer({
      above: true,
      class: "cm6-empty-caret-layer",
      markers: view => view.state.doc.length === 0 && view.state.selection.main.empty
        ? RectangleMarker.forRange(view, "cm6-empty-caret", EditorSelection.cursor(0)) : [],
      update: (u, dom) => {
        if (u.docChanged || u.selectionSet || u.viewportChanged || u.geometryChanged || u.focusChanged) {
          // Restart the blink so the caret is solid right after a click or key.
          dom.style.animationName = dom.style.animationName === "cm6-caret-blink" ? "cm6-caret-blink2" : "cm6-caret-blink";
          return true;
        }
        return false;
      },
    }),
  ];
}
