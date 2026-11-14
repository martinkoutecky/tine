import { EditorView } from "@codemirror/view";
import { editorIn, type EditorDriver } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { installKeybindings } from "../keybindings";
import { initParser } from "../render/parse";
import { isBlockMoving, pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { Block } from "./Block";

beforeAll(async () => { await initParser(); });
let dispose = () => {};
let keys = () => {};
afterEach(() => { dispose(); keys(); resetStore(); document.body.innerHTML = ""; });

/** [anchor, head] of the range the browser shows and later reports back to the
 *  editor. A textarea keeps its own; a CodeMirror editor's is the DOM range. */
function shownRange(editor: EditorDriver): [number, number] {
  const [start, end] = editor.selection();
  if (editor.kind !== "cm6") return editor.direction() === "backward" ? [end, start] : [start, end];
  const view = EditorView.findFromDOM(editor.element.closest<HTMLElement>(".cm-editor")!)!;
  const range = document.getSelection()!;
  return [view.posAtDOM(range.anchorNode!, range.anchorOffset), view.posAtDOM(range.focusNode!, range.focusOffset)];
}

// master fab3819c5: a sibling reorder must restore focus and selection inside
// the key gesture itself; a frame later Android has already dismissed the IME.
describe("sibling move keeps the editor inside the gesture", () => {
  for (const [dir, key] of [[-1, "ArrowUp"], [1, "ArrowDown"]] as const) {
    for (const direction of ["forward", "backward"] as const) {
      it(`${key} ${direction}`, async () => {
        loadSingle({ name: "Probe", kind: "page", title: "Probe", pre_block: null, blocks: [
          { id: "a", raw: "first block", collapsed: false, children: [] },
          { id: "b", raw: "second block", collapsed: false, children: [] },
          { id: "c", raw: "third block", collapsed: false, children: [] },
        ] });
        keys = installKeybindings();
        startEditing("b", 0);
        const root = document.createElement("div");
        document.body.append(root);
        dispose = render(() => <For each={pageByName("Probe")?.roots ?? []}>{(id) => <Block id={id} />}</For>, root);
        const editor = editorIn(root);
        editor.focus();
        editor.select(2, 8, direction);
        editor.keydown({ key, altKey: true, shiftKey: true, bubbles: true, cancelable: true });
        // Synchronously, before any animation frame.
        expect(doc.pages[0].roots).toEqual(dir < 0 ? ["b", "a", "c"] : ["a", "c", "b"]);
        expect(document.activeElement).toBe(editor.element);
        expect([editor.selection()[0], editor.selection()[1], editor.direction()]).toEqual([2, 8, direction]);
        // The browser's range too: the reorder cleared it, and its selectionchange may arrive after
        // the next frame, when the editor would read the cleared range back (a frame already pending
        // at the move collapsed the selection to the block start).
        const shown = direction === "backward" ? [8, 2] : [2, 8];
        expect(shownRange(editor)).toEqual(shown);
        await vi.waitFor(() => expect(isBlockMoving()).toBe(false));
        document.dispatchEvent(new Event("selectionchange"));
        expect(document.activeElement).toBe(editor.element);
        expect([editor.selection()[0], editor.selection()[1], editor.direction()]).toEqual([2, 8, direction]);
        expect(shownRange(editor)).toEqual(shown);
      });
    }
  }
});
