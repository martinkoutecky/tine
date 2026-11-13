import { maybeEditorIn } from "./tests/editorDriver";
// R2 lane B1: keyboard block commands (GH #576, #448, #379, #634, #525, #633,
// #513). Each assertion is the user outcome (clipboard text, tree shape, search
// field contents), reached the way a user reaches it: the palette entry or a
// chord the user bound in Settings.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { doc } from "./document/model";
import { extendSelectionTo, loadFeed, pageByName, resetStore, selectBlock, undo } from "./document";
import { clearClipboardPayload } from "./clipboard";
import { BUILTIN_KEYS, commandDefaults, installKeybindings, paletteCommands } from "./keybindings";
import { closeSwitcher, switcherOpen, switcherPrefill } from "./ui";
import { editingId, endEdit, startEditing } from "./editorController";
import { pasteAsLinkEdit, removeAuthorTarget } from "./blockRefActions";
import { setGraphMeta } from "./graphSession";
import { openPage, resetTabsToJournals } from "./router";
import { setToasts, toasts } from "./toasts";
import { bindTestHost } from "./document/host/wiring.test.support";
import type { BlockDto } from "./types";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { Block } from "./components/Block";
import * as wiring from "./document/host/wiring";
import * as model from "./document/model";

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
  clearClipboardPayload();
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
      "editor/collapse-children", "editor/expand-children", "editor/paste-as-link", "go/search-selection",
      ...Array.from({ length: 10 }, (_, n) => `editor/set-depth-${n}`)]) {
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
      const editor = maybeEditorIn(root)!;
      editor.element.focus();
      editor.select(5, 5);
      editor.keydown(new KeyboardEvent("keydown", { key: "U", code: "KeyU", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(pageByName("P")!.roots).toHaveLength(3));
      expect(doc.byId[A].raw).toBe(`Alpha\nid:: ${A}`);
      expect(maybeEditorIn(root)?.value()).toBe("Alpha");
      expect(editingId()).toBe(pageByName("P")!.roots[0]);
      maybeEditorIn(root)!.reportBlur();
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

// GH #794: Set depth N shows the block down to level N: levels 0..N-1 open, level N
// folded, deeper levels untouched (hidden by the fold above them).
describe("Set depth 0 to 9 (GH #794)", () => {
  const D = "66666666-6666-4666-8666-666666666666";
  const E = "77777777-7777-4777-8777-777777777777";
  // A (0) > C1 (1) > G1 (2) > D (3) > E (4, leaf);  A > C2 (1, leaf).
  // Everything with children starts as `fold` says; D is open on purpose.
  const deepTree = (format: "md" | "org", fold: { a: boolean; c1: boolean; g1: boolean; d: boolean }) =>
    loadSingle({ name: "P", kind: "page", title: "P", format, pre_block: null,
      blocks: [block(A, "Alpha", [
        block(C1, "One", [block(G1, "Grand", [block(D, "Deep", [block(E, "Leaf")], fold.d)], fold.g1)], fold.c1),
        block(C2, "Two"),
      ], fold.a), block(B, "Beta")] });
  const states = () => [A, C1, G1, D].map((id) => doc.byId[id].collapsed);
  const rawOf = (id: string) => doc.byId[id].raw;

  it("registers ten palette commands and no default chord", () => {
    for (let n = 0; n <= 9; n++) {
      const command = paletteCommands().find((c) => c.id === `editor/set-depth-${n}`);
      expect(command?.label, `set depth ${n}`).toBe(`Set depth ${n}`);
    }
  });

  it.each(["md", "org"] as const)("Set depth 2 matches the issue's test case in %s: opens the block and its children, folds the grandchildren, leaves deeper blocks alone", (format) => {
    deepTree(format, { a: true, c1: true, g1: false, d: false });
    const deepBefore = [rawOf(D), rawOf(E)];
    selectBlock(A);
    run("editor/set-depth-2");
    expect(states()).toEqual([false, false, true, false]);
    expect(rawOf(A)).not.toContain("collapsed");
    expect(rawOf(C1)).not.toContain("collapsed");
    expect(rawOf(G1)).toContain("collapsed");
    expect([rawOf(D), rawOf(E)]).toEqual(deepBefore); // hidden descendants are not written
  });

  it("Set depth 2 leaves a deep folded block folded, and deeper expanded ones expanded", () => {
    deepTree("md", { a: false, c1: false, g1: true, d: true });
    const deepBefore = [rawOf(D), rawOf(E)];
    selectBlock(A);
    run("editor/set-depth-2");
    expect(states()).toEqual([false, false, true, true]);
    expect([rawOf(D), rawOf(E)]).toEqual(deepBefore); // D was folded and hidden: no write
  });

  it("Set depth 0 folds the block, Set depth 1 opens it and folds its children, Set depth 9 opens every level", () => {
    deepTree("md", { a: false, c1: false, g1: false, d: false });
    selectBlock(A);
    run("editor/set-depth-0");
    expect(states()).toEqual([true, false, false, false]);
    run("editor/set-depth-1");
    expect(states()).toEqual([false, true, false, false]);
    run("editor/set-depth-9");
    expect(states()).toEqual([false, false, false, false]);
    run("editor/set-depth-3");
    expect(states()).toEqual([false, false, false, true]);
    run("editor/set-depth-4"); // E is a leaf: nothing at level 4 can fold
    expect(states()).toEqual([false, false, false, false]);
    expect(doc.byId[E].collapsed).toBe(false);
  });

  it("writes nothing for blocks already in the right state, and leaves one undo step per change", () => {
    deepTree("md", { a: false, c1: false, g1: false, d: false });
    selectBlock(A);
    run("editor/set-depth-2");
    expect(states()).toEqual([false, false, true, false]);
    const dirty = vi.spyOn(backend(), "writeRich");
    run("editor/set-depth-2"); // already there: no write and no undo entry
    expect(dirty).not.toHaveBeenCalled();
    undo(); // reverts the first press as ONE step, not the no-op
    expect(states()).toEqual([false, false, false, false]);
  });

  it("leaves a block with no children alone", () => {
    deepTree("md", { a: false, c1: false, g1: false, d: false });
    selectBlock(B);
    run("editor/set-depth-0");
    run("editor/set-depth-3");
    expect(doc.byId[B].collapsed).toBe(false);
    expect(rawOf(B)).not.toContain("collapsed");
  });

  it("acts on each selected block, measured from the topmost ones", () => {
    deepTree("md", { a: false, c1: false, g1: false, d: false });
    selectBlock(A);
    extendSelectionTo(C2); // A, C1, G1, D, E, C2 are selected: only A is a root
    run("editor/set-depth-1");
    expect(states()).toEqual([false, true, false, false]); // G1 and D are not roots of their own
    selectBlock(C1);
    extendSelectionTo(B); // C1 is folded now: A > C1, C2, B are visible; roots C1, C2, B
    run("editor/set-depth-2");
    expect(states()).toEqual([false, false, false, true]); // from C1: C1 and G1 open, D folded
  });

  it("works on the block being edited", () => {
    deepTree("md", { a: true, c1: true, g1: true, d: false });
    startEditing(A, 0);
    run("editor/set-depth-2");
    expect(states()).toEqual([false, false, true, false]);
  });

  it("with nothing selected or edited, says so and changes nothing", () => {
    deepTree("md", { a: true, c1: true, g1: true, d: false });
    run("editor/set-depth-2");
    expect(states()).toEqual([true, true, true, false]);
    expect(toasts().map((t) => t.kind)).toContain("warn");
  });
});

describe("search for the selected text (GH #633)", () => {
  it("opens search with the editor's selected text filled in", () => {
    const area: HTMLTextAreaElement = document.createElement("textarea");
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

describe("Remove author (GH #918)", () => {
  const authorBlock = (id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({
    ...block(id, raw, children, collapsed), properties: [["tine.author", "Ada Test"]],
  });
  const authored = `Agent words\ntine.author:: Ada Test\nid:: ${A}`;
  const loadAuthored = () => loadSingle({
    name: "P", kind: "page", title: "P", format: "md", pre_block: null,
    // Loaded blocks carry the backend's parsed properties, as in the app.
    blocks: [{ ...block(A, authored), properties: [["tine.author", "Ada Test"], ["id", A]] },
      { ...block(B, `Mine\nid:: ${B}`), properties: [["id", B]] }],
  });
  const offered = () => paletteCommands().some((c) => c.id === "editor/remove-author");

  it("checks availability without revisiting an unchanged 1,000-block outline", () => {
    const blocks = Array.from({ length: 1000 }, (_, i) => ({
      ...block(`cost-${i}`, `Words ${i}\ntine.author:: Ada Test`),
      properties: [["tine.author", "Ada Test"]] as [string, string][],
    }));
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null, blocks });
    let visits = 0;
    selectBlock("cost-0", { roots: blocks.map(b => b.id), collapsed: (_id, stored) => { visits++; return stored; } });
    expect(removeAuthorTarget()).toBe("cost-0"); // warm the selection memo
    visits = 0;
    expect(removeAuthorTarget()).toBe("cost-0");
    expect(removeAuthorTarget()).toBe("cost-0");
    expect(visits).toBe(0);
    extendSelectionTo("cost-1");
    expect(removeAuthorTarget()).toBe("cost-0");
    selectBlock("cost-1");
    expect(removeAuthorTarget()).toBe("cost-1");
  });

  it("ships without a key, like the other block commands", () => {
    expect(commandDefaults().find((c) => c.id === "editor/remove-author")).toMatchObject({ binding: "", label: "Remove author" });
  });

  it("removes the selected block's author from the palette, one undo step", () => {
    loadAuthored();
    selectBlock(A);
    run("editor/remove-author");
    expect(doc.byId[A].raw).toBe(`Agent words\nid:: ${A}`);
    expect(doc.byId[B].raw).toBe(`Mine\nid:: ${B}`);
    undo();
    expect(doc.byId[A].raw).toBe(authored);
  });

  it("is offered only where the block context menu offers Remove author", () => {
    loadAuthored();
    expect(offered(), "no block in play").toBe(false);
    selectBlock(B);
    expect(offered(), "a block without an author").toBe(false);
    selectBlock(A);
    extendSelectionTo(B);
    expect(offered(), "mixed selected blocks").toBe(true);
    selectBlock(A);
    expect(offered(), "one authored block").toBe(true);
  });

  it("works from a chord the user binds, while the authored block is being edited", () => {
    loadAuthored();
    startEditing(A, 0);
    const dispose = installKeybindings({ "editor/remove-author": "alt+shift+u" });
    try {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "U", code: "KeyU", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      expect(doc.byId[A].raw).toBe(`Agent words\nid:: ${A}`);
      expect(editingId()).toBeNull();
    } finally {
      dispose();
    }
  });

  it.each(["palette", "bound key"])("removes authored descendants from a routed page via %s, with one undo", (entry) => {
    const raw = (id: string) => `Words ${id}\ntine.author:: Ada Test\nkeep:: value`;
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null,
      blocks: [authorBlock(A, raw(A), [authorBlock(C1, raw(C1), [authorBlock(G1, raw(G1))]), block(C2, "Unauthored\nkeep:: child")], true),
        { ...block(B, "Outside\ntine.author:: Other"), properties: [["tine.author", "Other"]] }] });
    openPage("P", "page");
    selectBlock(A);
    const dispose = installKeybindings({ "editor/remove-author": "alt+shift+u" });
    try {
      if (entry === "palette") run("editor/remove-author");
      else window.dispatchEvent(new KeyboardEvent("keydown", { key: "U", code: "KeyU", altKey: true, shiftKey: true, bubbles: true, cancelable: true }));
      for (const id of [A, C1, G1]) expect(doc.byId[id].raw).toBe(`Words ${id}\nkeep:: value`);
      expect(doc.byId[C2].raw).toBe("Unauthored\nkeep:: child");
      expect(doc.byId[B].raw).toBe("Outside\ntine.author:: Other");
      undo();
      for (const id of [A, C1, G1]) expect(doc.byId[id].raw).toBe(raw(id));
    } finally { dispose(); }
  });

  it.each([1, 200])("submits one batched page edit for %i authored blocks", async (count) => {
    const blocks = Array.from({ length: count }, (_, i) => authorBlock(`bulk-${i}`, `Words ${i}\ntine.author:: Ada Test`));
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null, blocks });
    selectBlock(blocks[0].id); extendSelectionTo(blocks.at(-1)!.id);
    const dirty = vi.spyOn(wiring, "markDirty");
    const submit = vi.spyOn(backend(), "pageSubmit");
    const publish = vi.spyOn(model, "setDoc");
    const started = performance.now();
    run("editor/remove-author");
    expect(performance.now() - started).toBeLessThan(500);
    expect(publish).toHaveBeenCalledTimes(1);
    expect(dirty).toHaveBeenCalledTimes(1);
    expect(dirty).toHaveBeenCalledWith("P", "save-block");
    await vi.waitFor(() => expect(submit).toHaveBeenCalledTimes(1));
    for (const b of blocks) expect(doc.byId[b.id].raw).toBe(b.raw.replace("\ntine.author:: Ada Test", ""));
    undo();
    for (const b of blocks) expect(doc.byId[b.id].raw).toBe(b.raw);
  });

  it("offers a parent without attribution when a descendant has it, but skips read-only pages", () => {
    loadSingle({ name: "P", kind: "page", title: "P", format: "md", pre_block: null,
      blocks: [block(A, "Parent", [authorBlock(C1, "Child\ntine.author:: Ada Test")])] });
    selectBlock(A);
    expect(offered()).toBe(true);
    run("editor/remove-author");
    expect(offered()).toBe(false);
    loadSingle({ name: "Read only", kind: "page", title: "Read only", format: "md", pre_block: null, read_only: true,
      blocks: [authorBlock(B, authored)] });
    selectBlock(B);
    expect(offered()).toBe(false);
  });

  it("skips a read-only page in a mixed selection and batches each writable page", () => {
    const page = (name: string, id: string, read_only = false) => ({ name, kind: "page" as const, title: name, format: "md" as const,
      pre_block: null, read_only, blocks: [authorBlock(id, authored)] });
    loadFeed([page("P", A), page("Q", B, true), page("R", C1)]);
    selectBlock(A); extendSelectionTo(C1);
    const dirty = vi.spyOn(wiring, "markDirty");
    run("editor/remove-author");
    expect(doc.byId[A].raw).toBe(`Agent words\nid:: ${A}`);
    expect(doc.byId[C1].raw).toBe(`Agent words\nid:: ${A}`);
    expect(doc.byId[B].raw).toBe(authored);
    expect(dirty.mock.calls).toEqual([["P", "save-block"], ["R", "save-block"]]);
    undo();
    for (const id of [A, B, C1]) expect(doc.byId[id].raw).toBe(authored);
  });
});
