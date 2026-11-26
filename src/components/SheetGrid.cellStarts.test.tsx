// GH #874: an edit a key starts in a grid cell continues in that cell. Configured shortcuts
// for block commands run before the editor's own key handler and start an edit that names
// no editor; history restores the captured caret and selection.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { editingId, editingOwner, endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { cellSel, resetCellSelectionForTests } from "../sheet/selection";
import { maybeEditorIn, type EditorDriver } from "../tests/editorDriver";

beforeAll(async () => {
  await initParser();
});

const disposers: (() => void)[] = [];
afterEach(() => {
  for (const dispose of disposers.splice(0)) dispose();
  endEdit("page-navigation");
  resetCellSelectionForTests();
  resetStore();
  document.body.innerHTML = "";
});

const owner = "sheet:main:grid:0:0";
const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["grid"], format: "md", readOnly: false, guide: false };
const node = (id: string, raw: string, parent: string | null, children: string[] = []): StoreNode =>
  ({ id, raw, collapsed: false, parent, page: "Sheet", children });

function load(childRaw = "Child"): void {
  setDoc({
    byId: {
      grid: node("grid", "Grid\ntine.view:: grid", null, ["r1"]),
      r1: node("r1", "", "grid", ["c1", "c2"]),
      c1: node("c1", "Alpha", "r1", ["kid"]),
      kid: node("kid", childRaw, "c1"),
      c2: node("c2", "Beta", "r1"),
    },
    pages: [page], feed: ["Sheet"], loaded: true,
  });
}

function mount(): HTMLDivElement {
  const root = document.createElement("div");
  document.body.append(root);
  disposers.push(render(() => <Block id="grid" />, root));
  disposers.push(installKeybindings({
    "editor/insert-child-block": "alt+i", "editor/duplicate-block-above": "alt+u", "editor/duplicate-block": "alt+d",
  }));
  return root;
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const cell = (root: HTMLElement) => root.querySelector<HTMLElement>('.sheet-cell[data-row="0"][data-col="0"]')!;

/** The focused editor, which must lie inside `scope`; null when no editor is focused there. */
function focusedIn(scope: HTMLElement): EditorDriver | null {
  const editor = maybeEditorIn(scope);
  return editor && editor.element.contains(document.activeElement) ? editor : null;
}

async function edit(root: HTMLElement, id: "c1" | "kid", at: number): Promise<EditorDriver> {
  if (id === "c1") cell(root).dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
  else root.querySelector('[data-sheet-block-id="kid"]')!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  await tick();
  const editor = focusedIn(cell(root))!;
  expect(editingId()).toBe(id);
  editor.select(at);
  return editor;
}

describe("a configured block-command shortcut pressed in a grid cell", () => {
  it.each([
    ["Insert Child Block", "c1", "Child", "i", (c1: string[]) => c1.at(-1)!],
    ["Insert Child Block", "kid", "Child", "i", () => doc.byId.kid.children[0]],
    ["Duplicate Above", "kid", "Child", "u", (c1: string[]) => c1[0]],
    ["Duplicate Below", "kid", "", "d", (c1: string[]) => c1[1]],
  ] as const)("%s from %s", async (_name, from, childRaw, key, created) => {
    load(childRaw);
    const root = mount();
    const editor = await edit(root, from, 0);
    editor.keydown({ key, code: `Key${key.toUpperCase()}`, altKey: true });
    await tick();
    const id = created([...doc.byId.c1.children]);
    expect(Object.keys(doc.byId)).toHaveLength(6);
    expect(id && id !== "kid" && id !== "c1").toBe(true);
    expect(editingId()).toBe(id);
    const next = focusedIn(cell(root));
    if (editor.kind === "cm6") {
      // CodeMirror leaves a key a global command took before its own handler; the new
      // block gets no editor, as before this change.
      expect(editingOwner()).toBeNull();
      expect(next).toBeNull();
      return;
    }
    expect(editingOwner()).toBe(owner);
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: "grid", row: 0, col: 0 });
    next!.type("q");
    expect(doc.byId[id].raw).toBe(key === "i" ? "q" : `${childRaw}q`);
  });
});

describe("Redo in the same grid cell", () => {
  it("restores the selection range Undo left", async () => {
    load();
    const root = mount();
    const editor = await edit(root, "c1", 5);
    editor.type("x");
    editor.select(1, 3);
    editor.keydown({ key: "z", ctrlKey: true });
    await tick();
    expect(doc.byId.c1.raw).toBe("Alpha");
    focusedIn(cell(root))!.keydown({ key: "z", ctrlKey: true, shiftKey: true });
    await tick();
    expect(doc.byId.c1.raw).toBe("Alphax");
    expect(editingOwner()).toBe(owner);
    expect(focusedIn(cell(root))!.selection()).toEqual([1, 3]);
  });
});
