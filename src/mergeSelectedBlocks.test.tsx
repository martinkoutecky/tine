// GH #764 (D-20261101-L5): "Merge selected blocks" joins a block selection into
// its first block with line breaks, keeps every child under it, never loses a
// referenced block ID, and is one undo step. Each assertion is the outcome a
// user sees (the block tree, the toast) or what the window hands the host to
// write (the submitted page), reached through the palette command.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { loadFeed, loadSingle } from "./document/workingSet";
import { doc } from "./document/model";
import { extendSelectionTo, flushAll, hasSelection, resetStore, selectBlock, selectedIds, undo } from "./document";
import { commandDefaults, paletteCommands } from "./keybindings";
import { installReferenceChangeCounter, setToasts, toasts } from "./toasts";
import { bindTestHost, submittedPages } from "./document/host/wiring.test.support";
import type { BlockDto } from "./types";

const block = (id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({ id, raw, children, collapsed });
// What the host ships for a loaded block: the parser's property facets come with it.
const withProps = (b: BlockDto, properties: [string, string][]): BlockDto => ({ ...b, properties });
const ID1 = "aaaaaaaa-0000-4000-8000-000000000001";
const ID2 = "aaaaaaaa-0000-4000-8000-000000000002";
const counts = new Map<string, number>();

function merge(): void {
  const command = paletteCommands().find((c) => c.id === "editor/merge-selected-blocks");
  expect(command, "the palette offers Merge selected blocks").toBeDefined();
  command!.run();
}
const load = (blocks: BlockDto[], format: "md" | "org" = "md") =>
  loadSingle({ name: "P", kind: "page", title: "P", id: "pages/P.md", rev: "r1", format, pre_block: null, blocks });
const select = (from: string, to: string) => { selectBlock(from); extendSelectionTo(to); };
const snap = () => JSON.stringify({
  roots: doc.pages.map((p) => [p.name, p.roots]),
  blocks: Object.values(doc.byId).map((n) => [n.id, n.raw, n.parent, n.children]).sort(),
});
const lastToast = () => toasts().at(-1)?.message ?? "";

beforeAll(() => initParser());
beforeEach(async () => {
  counts.clear();
  setToasts([]);
  installReferenceChangeCounter((ids) => ids.reduce((sum, id) => sum + (counts.get(id) ?? 0), 0));
  await bindTestHost();
});
afterEach(() => { resetStore(); setToasts([]); vi.restoreAllMocks(); });

describe("the command (GH #764)", () => {
  it("is palette and keymap only: no default key", () => {
    expect(commandDefaults().find((c) => c.id === "editor/merge-selected-blocks")?.binding).toBe("");
  });

  it("asks for two blocks when fewer are selected", () => {
    load([block("a", "one"), block("b", "two")]);
    merge();
    expect(lastToast()).toContain("Select two or more blocks");
    selectBlock("a");
    merge();
    expect(doc.byId.b?.raw).toBe("two");
    expect(lastToast()).toContain("Select two or more blocks");
  });
});

describe("merging a selection (GH #764)", () => {
  it("joins the texts with line breaks, keeps all children in order, and saves one block", async () => {
    const submit = vi.spyOn(backend(), "pageSubmit");
    load([
      block("a", "one", [block("a1", "one child")]),
      block("b", "two", [block("b1", "two child")]),
      block("c", "three", [block("c1", "three child")]),
      block("d", "after"),
    ]);
    select("a", "c1");
    // The selection is a, a1, b, b1, c, c1 in visible order.
    expect(selectedIds()).toEqual(["a", "a1", "b", "b1", "c", "c1"]);
    merge();
    expect(doc.byId.a.raw).toBe("one\none child\ntwo\ntwo child\nthree\nthree child");
    expect(doc.pages[0].roots).toEqual(["a", "d"]);
    expect(doc.byId.a.children).toEqual([]);
    expect(Object.keys(doc.byId).sort()).toEqual(["a", "d"]);
    // The merged block stays selected, and can be edited as usual.
    expect(selectedIds()).toEqual(["a"]);
    // The text handed to the host for the file.
    await flushAll();
    const page = submittedPages(submit, "P").at(-1)!;
    expect(page.blocks.map((b) => b.raw)).toEqual(["one\none child\ntwo\ntwo child\nthree\nthree child", "after"]);
  });

  it("merges the complete selected parent subtrees, including endpoint descendants (GH #941)", () => {
    load([
      block("a", "one", [block("a1", "own child")]),
      block("b", "two", [block("b1", "kept child", [block("b11", "grandchild")])]),
    ]);
    select("a", "b");
    expect(selectedIds()).toEqual(["a", "a1", "b", "b1", "b11"]);
    merge();
    expect(doc.byId.a.raw).toBe("one\nown child\ntwo\nkept child\ngrandchild");
    expect(doc.byId.a.children).toEqual([]);
    expect(Object.keys(doc.byId)).toEqual(["a"]);
  });

  it("is one undo step", () => {
    load([block("a", "one"), block("b", "two", [block("b1", "kid")]), block("c", "three")]);
    const before = snap();
    select("a", "c");
    merge();
    expect(doc.byId.a.raw).toBe("one\ntwo\nkid\nthree");
    undo();
    expect(snap()).toBe(before);
  });

  it("merges every selected descendant's text even under a collapsed parent (GH #941)", () => {
    load([block("a", "one"), block("b", "two", [block("b1", "hidden kid")], true)]);
    select("a", "b");
    expect(selectedIds()).toEqual(["a", "b", "b1"]);
    merge();
    expect(doc.byId.a.raw).toBe("one\ntwo\nhidden kid");
    expect(doc.byId.a.children).toEqual([]);
    expect(doc.byId.b1).toBeUndefined();
  });

  it("joins Org blocks with a line break too", () => {
    load([block("a", "one"), block("b", "two")], "org");
    select("a", "b");
    merge();
    expect(doc.byId.a.raw).toBe("one\ntwo");
  });

  it("merges blocks at different levels", () => {
    load([block("a", "one", [block("a1", "child")]), block("b", "sibling of one")]);
    select("a1", "b");
    merge();
    expect(doc.byId.a1.raw).toBe("child\nsibling of one");
    expect(doc.pages[0].roots).toEqual(["a"]);
  });
});

describe("referenced block IDs are never lost (GH #764)", () => {
  it("moves the one referenced ID of a merged block to the first block", () => {
    load([block("a", "one"), block("b", `two\nid:: ${ID1}`)]);
    counts.set(ID1, 3);
    select("a", "b");
    merge();
    expect(doc.byId.a.raw).toBe(`one\ntwo\nid:: ${ID1}`);
    expect(toasts().at(-1)?.message).toBe("3 references now point to this block");
  });

  it("refuses, changing nothing, when two blocks carry a referenced ID", () => {
    load([block("a", `one\nid:: ${ID1}`), block("b", `two\nid:: ${ID2}`)]);
    counts.set(ID2, 1);
    const before = snap();
    select("a", "b");
    merge();
    expect(snap()).toBe(before);
    expect(lastToast()).toContain("referenced by its block ID");
    expect(hasSelection()).toBe(true);
  });

  it("refuses when two later blocks are both referenced, even if the first has no ID", () => {
    load([block("a", "one"), block("b", `two\nid:: ${ID1}`), block("c", `three\nid:: ${ID2}`)]);
    counts.set(ID1, 1);
    counts.set(ID2, 1);
    const before = snap();
    select("a", "c");
    merge();
    expect(snap()).toBe(before);
    expect(lastToast()).toContain("referenced by its block ID");
  });

  it("drops an ID nothing refers to, as Backspace does", () => {
    load([block("a", `one\nid:: ${ID1}`), block("b", `two\nid:: ${ID2}`)]);
    select("a", "b");
    merge();
    expect(doc.byId.a.raw).toBe(`one\ntwo\nid:: ${ID1}`);
  });
});

describe("what a merge refuses (GH #764)", () => {
  const unchanged = (run: () => void) => {
    const before = snap();
    run();
    expect(snap()).toBe(before);
  };

  it.each([
    ["a query", "{{query (todo todo)}}"],
    ["a page embed", "{{embed [[Other]]}}"],
    ["a block embed", `{{embed ((${ID1}))}}`],
    ["a PDF highlight", "Important passage\nhl-page:: 42\nhl-color:: yellow\nls-type:: annotation"],
  ])("refuses a selection holding %s", (_name, raw) => {
    const props = raw.includes("annotation") ? [["hl-page", "42"], ["hl-color", "yellow"], ["ls-type", "annotation"]] as [string, string][] : [];
    load([block("a", "one"), withProps(block("b", raw), props)]);
    unchanged(() => { select("a", "b"); merge(); });
    expect(lastToast()).toContain("query, an embed, a sheet or a PDF highlight");
  });

  it("refuses blocks from two pages", () => {
    loadFeed([
      { name: "P", kind: "page", title: "P", id: "pages/P.md", rev: "r1", format: "md", pre_block: null, blocks: [block("a", "one")] },
      { name: "Q", kind: "page", title: "Q", id: "pages/Q.md", rev: "r2", format: "md", pre_block: null, blocks: [block("b", "two")] },
    ]);
    unchanged(() => { select("a", "b"); merge(); });
    expect(lastToast()).toContain("more than one page");
  });

  it("refuses a read-only page", () => {
    loadSingle({ name: "P", kind: "page", title: "P", id: "pages/P.md", rev: "r1", format: "md", pre_block: null,
      read_only: true, blocks: [block("a", "one"), block("b", "two")] });
    unchanged(() => { select("a", "b"); merge(); });
    expect(lastToast()).toContain("read-only");
  });
});
