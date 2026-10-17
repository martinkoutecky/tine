// R2 lane B1: keyboard block commands (GH #576, #448, #379, #634, #525, #633,
// #513). Each assertion is the user outcome (clipboard text, tree shape, search
// field contents), reached the way a user reaches it: the palette entry or a
// chord the user bound in Settings.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { doc } from "./document/model";
import { extendSelectionTo, pageByName, resetStore, selectBlock, undo } from "./document";
import { clearClipboardSlot } from "./clipboard";
import { BUILTIN_KEYS, commandDefaults, installKeybindings, paletteCommands } from "./keybindings";
import { closeSwitcher, switcherOpen, switcherPrefill } from "./ui";
import { editingId, endEdit, startEditing } from "./editorController";
import { pasteAsLinkEdit } from "./blockRefActions";
import { setGraphMeta } from "./graphSession";
import { openPage, resetTabsToJournals } from "./router";
import { setToasts } from "./toasts";
import { bindTestHost } from "./document/host/wiring.test.support";
import type { BlockDto } from "./types";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { Block } from "./components/Block";

const A = "11111111-1111-4111-8111-111111111111";
const B = "22222222-2222-4222-8222-222222222222";
const C1 = "33333333-3333-4333-8333-333333333333";
const C2 = "44444444-4444-4444-8444-444444444444";
const G1 = "55555555-5555-4555-8555-555555555555";
const block = (id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({ id, raw, children, collapsed });

function run(id: string): void {
  const command = paletteCommands().find((c) => c.id === id);
  expect(command, `palette offers ${id}`).toBeDefined();
  command!.run();
}

beforeAll(() => initParser());
beforeEach(async () => {
  await bindTestHost();
  vi.spyOn(backend(), "writeRich").mockResolvedValue();
  vi.spyOn(backend(), "writeText").mockResolvedValue();
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
  setGraphMeta({ root: "/fixture" } as never);
});
afterEach(() => {
  resetTabsToJournals(); // a route left on page "P" would reload it over the next fixture
  endEdit("blur");
  closeSwitcher();
  clearClipboardSlot();
  resetStore();
  setGraphMeta(null);
  setToasts([]);
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

function loadTree() {
  loadSingle({
    name: "P", kind: "page", title: "P", format: "md", pre_block: null,
    blocks: [
      block(A, `Alpha\nid:: ${A}`, [
        block(C1, `Child one\nid:: ${C1}`, [block(G1, `Grand\nid:: ${G1}`)]),
        block(C2, `Child two\nid:: ${C2}`),
      ]),
      block(B, `Beta\nid:: ${B}`),
    ],
  });
}
const firstLines = (ids: readonly string[]) => ids.map((id) => doc.byId[id].raw.split("\n")[0]);

describe("default bindings (R2 B1)", () => {
  it("adds no default binding and leaves every mod+digit chord on pane focus", () => {
    const defaults = Object.fromEntries(commandDefaults().map((c) => [c.id, c.binding]));
    for (const id of ["select/copy-block-ref", "editor/duplicate-block", "editor/insert-child-block",
      "editor/collapse-children", "editor/expand-children", "editor/paste-as-link", "go/search-selection"]) {
      expect(defaults[id], id).toBe("");
    }
    for (let n = 1; n <= 9; n++) expect(defaults[`pane/focus-${n}`]).toBe(`mod+${n}`);
    const digitChords = commandDefaults().filter((c) => /^mod\+[1-9]$/.test(c.binding)).map((c) => c.id);
    expect(digitChords.sort()).toEqual(Array.from({ length: 9 }, (_, i) => `pane/focus-${i + 1}`));
  });

  it("documents the plain-paste key in the shortcut reference (GH #513, OG mod+shift+v)", () => {
    const row = BUILTIN_KEYS.find((k) => k.id === "builtin/editor/paste-plain");
    expect(row).toMatchObject({ binding: "mod+shift+v", scope: "editor" });
  });
});

describe("copy block refs of the selection (GH #576)", () => {
  it("copies one ref per selected block, in visible order, with nothing in edit mode", async () => {
    loadTree();
    selectBlock(A);
    extendSelectionTo(B);
    run("select/copy-block-ref");
    await vi.waitFor(() => expect(backend().writeText).toHaveBeenCalledTimes(1));
    expect(vi.mocked(backend().writeText).mock.calls[0][0]).toBe(
      [A, C1, G1, C2, B].map((id) => `- ((${id}))`).join("\n"),
    );
  });

  it("copies a bare ref for a single selected block", async () => {
    loadTree();
    selectBlock(B);
    run("select/copy-block-ref");
    await vi.waitFor(() => expect(backend().writeText).toHaveBeenCalledWith(`((${B}))`));
  });

  it("works from a chord the user binds, while a block is being edited", async () => {
    loadTree();
    startEditing(B, 0);
    const dispose = installKeybindings({ "select/copy-block-ref": "alt+r" });
    try {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "r", code: "KeyR", altKey: true, bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(backend().writeText).toHaveBeenCalledWith(`((${B}))`));
    } finally {
      dispose();
    }
  });
});

describe("duplicate block (GH #379)", () => {
  it("preserves the original mounted editor when inserting a copy above it", async () => {
    loadTree();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <For each={pageByName("P")!.roots}>{id => <Block id={id} />}</For>, root);
    const keys = installKeybindings({ "editor/duplicate-block-above": "alt+shift+u" });
    try {
      startEditing(A, 5);
      const editor = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
      editor.focus();
      editor.setSelectionRange(5, 5);
      editor.dispatchEvent(new KeyboardEvent("keydown", { key: "U", code: "KeyU", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(pageByName("P")!.roots).toHaveLength(3));
      expect(doc.byId[A].raw).toBe(`Alpha\nid:: ${A}`);
      expect(root.querySelector<HTMLTextAreaElement>("textarea.block-editor")?.value).toBe("Alpha");
      expect(editingId()).toBe(pageByName("P")!.roots[0]);
      root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!.dispatchEvent(new FocusEvent("blur"));
      expect(doc.byId[A].raw).toBe(`Alpha\nid:: ${A}`);
    } finally { keys(); dispose(); }
  });
  it("offers duplicate above without a default key, copying children with fresh ids and one Undo", async () => {
    expect(commandDefaults().find(c => c.id === "editor/duplicate-block-above")?.binding).toBe("");
    loadTree();
    startEditing(A, 0);
    run("editor/duplicate-block-above");
    await vi.waitFor(() => expect(pageByName("P")!.roots).toHaveLength(3));
    const roots = pageByName("P")!.roots;
    expect(roots[1]).toBe(A);
    const copy = roots[0];
    expect(copy).not.toBe(A);
    expect(editingId()).toBe(copy);
    expect(firstLines(doc.byId[copy].children)).toEqual(["Child one", "Child two"]);
    expect(firstLines(doc.byId[doc.byId[copy].children[0]].children)).toEqual(["Grand"]);
    const ids = [...JSON.stringify(Object.values(doc.byId).map(n => n.raw)).matchAll(/id:: ([0-9a-f-]{36})/g)].map(m => m[1]);
    expect(new Set(ids).size).toBe(ids.length);
    undo();
    expect(pageByName("P")!.roots).toEqual([A, B]);
  });

  it("duplicate above preserves an empty original and repeated presses stack above", async () => {
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null, blocks: [block(A, "")] });
    startEditing(A, 0);
    for (let count = 2; count <= 3; count++) {
      run("editor/duplicate-block-above");
      await vi.waitFor(() => expect(pageByName("P")!.roots).toHaveLength(count));
      expect(pageByName("P")!.roots[count - 1]).toBe(A);
      expect(editingId()).toBe(pageByName("P")!.roots[0]);
    }
  });

  it("duplicate above is bindable and keeps a selected subtree in order without copying descendants twice", async () => {
    loadTree();
    selectBlock(A);
    extendSelectionTo(B);
    const dispose = installKeybindings({ "editor/duplicate-block-above": "alt+shift+u" });
    try {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "U", code: "KeyU", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(pageByName("P")!.roots).toHaveLength(4));
      expect(firstLines(pageByName("P")!.roots)).toEqual(["Alpha", "Beta", "Alpha", "Beta"]);
      expect(pageByName("P")!.roots.slice(2)).toEqual([A, B]);
      expect(firstLines(doc.byId[pageByName("P")!.roots[0]].children)).toEqual(["Child one", "Child two"]);
    } finally { dispose(); }
  });

  it("duplicate above copies a nested Org block with its collapse state and distinct identity", async () => {
    loadSingle({ name: "P", kind: "page", title: "P", format: "org", pre_block: null,
      blocks: [block(A, "Parent", [block(B, "Child\n:PROPERTIES:\n:ID: " + B + "\n:collapsed: true\n:END:", [block(C1, "Grand")], true)])] });
    startEditing(B, 0);
    run("editor/duplicate-block-above");
    await vi.waitFor(() => expect(doc.byId[A].children).toHaveLength(2));
    const copy = doc.byId[A].children[0];
    expect(doc.byId[A].children[1]).toBe(B);
    expect(doc.byId[copy].parent).toBe(A);
    expect(doc.byId[copy].raw).not.toContain(B);
    expect(doc.byId[copy].collapsed).toBe(true);
    expect(firstLines(doc.byId[copy].children)).toEqual(["Grand"]);
    expect(editingId()).toBe(copy);
  });
  it("copies the block with its children directly below, with fresh ids, as one undo step", async () => {
    loadTree();
    selectBlock(A);
    run("editor/duplicate-block");
    await vi.waitFor(() => expect(pageByName("P")!.roots).toHaveLength(3));
    const roots = pageByName("P")!.roots;
    expect(firstLines(roots)).toEqual(["Alpha", "Alpha", "Beta"]);
    const copy = roots[1];
    expect(copy).not.toBe(A);
    expect(firstLines(doc.byId[copy].children)).toEqual(["Child one", "Child two"]);
    expect(firstLines(doc.byId[doc.byId[copy].children[0]].children)).toEqual(["Grand"]);
    // No block shares an identity with its original.
    const ids = [...JSON.stringify(Object.values(doc.byId).map((n) => n.raw)).matchAll(/id:: ([0-9a-f-]{36})/g)].map((m) => m[1]);
    expect(new Set(ids).size).toBe(ids.length);
    undo();
    expect(firstLines(pageByName("P")!.roots)).toEqual(["Alpha", "Beta"]);
  });

  it("duplicates an empty block instead of replacing it, and the caret follows the copy", async () => {
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null, blocks: [block(A, "")] });
    startEditing(A, 0);
    run("editor/duplicate-block");
    await vi.waitFor(() => expect(pageByName("P")!.roots).toHaveLength(2));
    expect(editingId()).toBe(pageByName("P")!.roots[1]);
  });

  it("stacks: pressing it again on the copy adds another below", async () => {
    loadTree();
    startEditing(B, 4);
    run("editor/duplicate-block");
    await vi.waitFor(() => expect(pageByName("P")!.roots).toHaveLength(3));
    expect(editingId()).toBe(pageByName("P")!.roots[2]);
    run("editor/duplicate-block");
    await vi.waitFor(() => expect(pageByName("P")!.roots).toHaveLength(4));
    expect(firstLines(pageByName("P")!.roots)).toEqual(["Alpha", "Beta", "Beta", "Beta"]);
  });
});

describe("insert child block (GH #634)", () => {
  it("appends an empty child under the edited block and edits it", () => {
    loadTree();
    startEditing(B, 0);
    run("editor/insert-child-block");
    const children = doc.byId[B].children;
    expect(children).toHaveLength(1);
    expect(doc.byId[children[0]].raw).toBe("");
    expect(editingId()).toBe(children[0]);
  });

  it("goes after the existing children and opens a collapsed block so the child shows", () => {
    loadSingle({
      name: "P", kind: "page", title: "P", format: "md", pre_block: null,
      blocks: [block(A, `Alpha\nid:: ${A}`, [block(C1, `One\nid:: ${C1}`)], true)],
    });
    selectBlock(A);
    run("editor/insert-child-block");
    expect(doc.byId[A].collapsed).toBe(false);
    expect(doc.byId[A].children).toHaveLength(2);
    expect(doc.byId[A].children[0]).toBe(C1);
    expect(editingId()).toBe(doc.byId[A].children[1]);
  });
});

describe("collapse / expand one level (GH #525)", () => {
  it.each(["md", "org"] as const)("GH #696: repeated presses skip open levels and leaves in %s, with one undo per press", (format) => {
    loadSingle({ name: "P", kind: "page", title: "P", format, pre_block: null,
      blocks: [block(A, "Parent", [block(C2, "Leaf"),
        block(C1, "Child", [block(G1, "Grand", [block(B, "Deep")], true)], true),
      ], true)] });
    selectBlock(A);
    run("editor/expand-children");
    expect(doc.byId[A].collapsed).toBe(false);
    expect(doc.byId[C1].collapsed).toBe(true);
    run("editor/expand-children");
    expect(doc.byId[C1].collapsed).toBe(false);
    expect(doc.byId[G1].collapsed).toBe(true);
    run("editor/expand-children");
    expect(doc.byId[G1].collapsed).toBe(false);
    undo();
    expect(doc.byId[G1].collapsed).toBe(true);
    expect(doc.byId[C1].collapsed).toBe(false);
    undo();
    expect(doc.byId[C1].collapsed).toBe(true);
    expect(doc.byId[A].collapsed).toBe(false);
    undo();
    expect(doc.byId[A].collapsed).toBe(true);
  });

  it.each(["md", "org"] as const)("GH #696: collapse folds the block itself once its children are folded, and never opens it, in %s", (format) => {
    loadSingle({ name: "P", kind: "page", title: "P", format, pre_block: null,
      blocks: [block(A, "Parent", [block(C2, "Leaf"),
        block(C1, "Child", [block(G1, "Grand")]),
      ])] });
    selectBlock(A);
    run("editor/collapse-children");
    expect(doc.byId[C1].collapsed).toBe(true);
    expect(doc.byId[A].collapsed).toBe(false);
    run("editor/collapse-children");
    expect(doc.byId[A].collapsed).toBe(true);
    run("editor/collapse-children");
    expect(doc.byId[A].collapsed).toBe(true);
    expect(doc.byId[C1].collapsed).toBe(true);
    undo();
    expect(doc.byId[A].collapsed).toBe(false);
    expect(doc.byId[C1].collapsed).toBe(true);
  });

  it("GH #696: collapse folds a block whose children are all leaves", () => {
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null,
      blocks: [block(A, "Parent", [block(C1, "Leaf one"), block(C2, "Leaf two")])] });
    selectBlock(A);
    run("editor/collapse-children");
    expect(doc.byId[A].collapsed).toBe(true);
  });

  it("GH #696: page scope advances past open roots and stops when everything is expanded", () => {
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null,
      blocks: [block(A, "Parent", [block(C1, "Child", [block(G1, "Leaf")], true)]), block(B, "Leaf")] });
    openPage("P", "page");
    run("editor/expand-children");
    expect(doc.byId[C1].collapsed).toBe(false);
    run("editor/expand-children");
    undo();
    expect(doc.byId[C1].collapsed).toBe(true);
  });

  it("collapses only the immediate children and leaves deeper folds and the block itself alone", () => {
    loadSingle({
      name: "P", kind: "page", title: "P", format: "md", pre_block: null,
      blocks: [block(A, `Alpha\nid:: ${A}`, [
        block(C1, `One\nid:: ${C1}`, [block(G1, `Grand\nid:: ${G1}`)]),
        block(C2, `Two\nid:: ${C2}`),
      ])],
    });
    selectBlock(A);
    run("editor/collapse-children");
    expect(doc.byId[C1].collapsed).toBe(true);
    expect(doc.byId[A].collapsed).toBe(false);
    expect(doc.byId[C1].raw).toContain("collapsed:: true");
    run("editor/expand-children");
    expect(doc.byId[C1].collapsed).toBe(false);
    undo();
    expect(doc.byId[C1].collapsed).toBe(true);
  });

  it("keeps a deliberately folded grandchild folded when its parent's level is expanded", () => {
    loadSingle({
      name: "P", kind: "page", title: "P", format: "md", pre_block: null,
      blocks: [block(A, `Alpha\nid:: ${A}`, [
        block(C1, `One\nid:: ${C1}`, [block(G1, `Grand\nid:: ${G1}`, [block("66666666-6666-4666-8666-666666666666", "Deep")], true)], true),
      ], true)],
    });
    selectBlock(A);
    run("editor/expand-children");
    expect(doc.byId[A].collapsed).toBe(false);
    expect(doc.byId[C1].collapsed).toBe(true);
    run("editor/expand-children");
    expect(doc.byId[C1].collapsed).toBe(false);
    expect(doc.byId[G1].collapsed).toBe(true);
  });

  it("with nothing selected, works on the page's top-level blocks", () => {
    loadTree();
    openPage("P", "page");
    run("editor/collapse-children");
    expect(doc.byId[A].collapsed).toBe(true);
    expect(doc.byId[B].collapsed).toBe(false); // B has no children: nothing to fold
    run("editor/expand-children");
    expect(doc.byId[A].collapsed).toBe(false);
  });
});

describe("search for the selected text (GH #633)", () => {
  it("opens search with the editor's selected text filled in", () => {
    const area = document.createElement("textarea");
    area.value = "find the  quick fox here";
    document.body.append(area);
    area.focus();
    area.setSelectionRange(9, 19);
    run("go/search-selection");
    expect(switcherOpen()).toBe(true);
    expect(switcherPrefill()).toBe("quick fox");
  });

  it("opens an empty search when nothing is selected", () => {
    run("go/search-selection");
    expect(switcherOpen()).toBe(true);
    expect(switcherPrefill()).toBe("");
  });
});

describe("paste as link (GH #448)", () => {
  const ref = "((cf1ef346-3c69-47f6-b09e-52b109ba6ee7))";
  it("wraps the selection around a URL or a block ref, bare without a selection, never for other text", () => {
    expect(pasteAsLinkEdit("see context here", 4, 11, "https://x.org/a", "md")?.text).toBe("see [context](https://x.org/a) here");
    expect(pasteAsLinkEdit("see context here", 4, 11, `  ${ref}\n`, "md")?.text).toBe(`see [context](${ref}) here`);
    expect(pasteAsLinkEdit("see context here", 4, 11, "https://x.org/a", "org")?.text).toBe("see [[https://x.org/a][context]] here");
    const bare = pasteAsLinkEdit("see ", 4, 4, "https://x.org/a", "md");
    expect(bare).toMatchObject({ text: "see https://x.org/a", start: 19, end: 19 });
    expect(pasteAsLinkEdit("see context", 4, 11, "just words", "md")).toBeNull();
  });
});
