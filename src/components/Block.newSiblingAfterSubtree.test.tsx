import { editorIn } from "../tests/editorDriver";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { For } from "solid-js";
import { pageByName, undo } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import { editingId, startEditing } from "../editorController";
import { commandDefaults, installKeybindings } from "../keybindings";
import { Block } from "./Block";
import { installBlockEditorLifecycle, mount, page } from "../tests/blockEditorTestkit";
import type { BlockDto } from "../types";

installBlockEditorLifecycle();

let uninstall = () => {};
beforeEach(() => { uninstall = installKeybindings(); });
afterEach(() => uninstall());

const b = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, children, collapsed: false });

// GH #818 (D-20261101-L5): Alt+Enter while editing makes a new block at the same
// level, after the block's whole subtree, and the caret goes there. The thread's
// own example: father 1 gets a new block before father 2; the inner son gets one
// after its children.
// The store gives loaded blocks fresh ids; `id(raw)` finds a block by its text.
const id = (raw: string): string => Object.values(doc.byId).find((n) => n.raw === raw)!.id;
function open(name: string, at: string, blocks: BlockDto[], caret = 0) {
  loadSingle(page(name, blocks));
  startEditing(id(at), caret);
  const { root, dispose } = mount(() => <For each={pageByName(name)?.roots ?? []}>{(rid) => <Block id={rid} />}</For>);
  return { editor: editorIn(root), dispose };
}
const altEnter = (editor: ReturnType<typeof editorIn>) => {
  editor.focus();
  return editor.keydown({ key: "Enter", altKey: true, bubbles: true, cancelable: true });
};
const tree = () => [
  b("f1", "father 1", [b("s1", "son 1", [b("ss", "son's son")]), b("s2", "son 2")]),
  b("f2", "father 2"),
];

describe("Alt+Enter: new block after the subtree (GH #818)", () => {
  it("is the default key of a rebindable editor command", () => {
    expect(commandDefaults().find((c) => c.id === "editor/new-sibling-after-subtree"))
      .toMatchObject({ binding: "alt+enter", scope: "editor" });
  });

  it("at a top-level block adds a sibling after its whole subtree, before the next block, and edits it", () => {
    const { editor, dispose } = open("Top", "father 1", tree(), 3);
    try {
      expect(altEnter(editor).defaultPrevented).toBe(true);
      const roots = pageByName("Top")!.roots;
      expect(roots).toHaveLength(3);
      expect(doc.byId[roots[0]].raw).toBe("father 1"); // not split at the caret
      expect(doc.byId[roots[2]].raw).toBe("father 2");
      expect(doc.byId[roots[1]].raw).toBe("");
      expect(doc.byId[roots[0]].children.map((k) => doc.byId[k].raw)).toEqual(["son 1", "son 2"]);
      expect(editingId()).toBe(roots[1]);
    } finally {
      dispose();
    }
  });

  it("at a nested block adds a sibling after that block's children, at its own level", () => {
    const { editor, dispose } = open("Nested", "son 1", tree());
    try {
      altEnter(editor);
      const father = pageByName("Nested")!.roots[0];
      const kids = doc.byId[father].children;
      expect(kids).toHaveLength(3);
      expect(doc.byId[kids[0]].raw).toBe("son 1");
      expect(doc.byId[kids[2]].raw).toBe("son 2");
      expect(doc.byId[kids[1]].raw).toBe("");
      expect(doc.byId[kids[0]].children.map((k) => doc.byId[k].raw)).toEqual(["son's son"]);
      expect(doc.byId[kids[1]].parent).toBe(father);
      expect(editingId()).toBe(kids[1]);
    } finally {
      dispose();
    }
  });

  it("keeps the text typed so far and the children where they are; one undo takes the new block away again", () => {
    const { editor, dispose } = open("Undo", "father 1", tree(), 8);
    try {
      editor.input("father 1 edited");
      altEnter(editor);
      const [first, fresh] = pageByName("Undo")!.roots;
      expect(doc.byId[first].raw).toBe("father 1 edited");
      expect(doc.byId[first].children).toHaveLength(2);
      expect(doc.byId[fresh].raw).toBe("");
      expect(pageByName("Undo")!.roots).toHaveLength(3);
      undo();
      expect(pageByName("Undo")!.roots).toHaveLength(2);
      // The typing before it is its own step, as with Enter.
      expect(doc.byId[pageByName("Undo")!.roots[0]].raw).toBe("father 1 edited");
      undo();
      expect(doc.byId[pageByName("Undo")!.roots[0]].raw).toBe("father 1");
    } finally {
      dispose();
    }
  });
});
