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
        const editor = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
        editor.focus();
        editor.setSelectionRange(2, 8, direction);
        editor.dispatchEvent(new KeyboardEvent("keydown", { key, altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
        // Synchronously, before any animation frame.
        expect(doc.pages[0].roots).toEqual(dir < 0 ? ["b", "a", "c"] : ["a", "c", "b"]);
        expect(document.activeElement).toBe(editor);
        expect([editor.selectionStart, editor.selectionEnd, editor.selectionDirection]).toEqual([2, 8, direction]);
        await vi.waitFor(() => expect(isBlockMoving()).toBe(false));
        expect(document.activeElement).toBe(editor);
        expect([editor.selectionStart, editor.selectionEnd, editor.selectionDirection]).toEqual([2, 8, direction]);
      });
    }
  }
});
