// Headless tests for the editing tree ops + caret tracking — the M0 logic that
// the prior Qt attempt got wrong (caret lost on indent/split/merge). No DOM
// needed; these are pure operations on the store.

import { describe, it, expect, beforeEach } from "vitest";
import {
  doc,
  resetStore,
  loadSingle,
  loadFeed,
  editingId,
  splitBlock,
  indentBlock,
  outdentBlock,
  mergeWithPrev,
  toggleCollapse,
  takeCaretFor,
  visibleOrder,
  startEditing,
  setRaw,
  undo,
  redo,
} from "./store";
import type { BlockDto, PageDto } from "./types";

let counter = 0;
function blk(raw: string, children: BlockDto[] = []): BlockDto {
  return { id: `t${counter++}`, raw, collapsed: false, children };
}
function load(blocks: BlockDto[]): PageDto {
  const dto: PageDto = { name: "Test", kind: "page", title: "Test", pre_block: null, blocks };
  loadSingle(dto);
  return dto;
}

/** Snapshot the tree as nested [raw, [children]] for easy assertions. */
function shape(ids: string[] = doc.pages[0].roots): any[] {
  return ids.map((id) => {
    const n = doc.byId[id];
    return n.children.length ? [n.raw, shape(n.children)] : [n.raw];
  });
}

beforeEach(() => {
  counter = 0;
  resetStore();
});

describe("split (Enter)", () => {
  it("splits a flat block into two siblings, caret at start of new", () => {
    const dto = load([blk("hello world")]);
    const id = dto.blocks[0].id;
    splitBlock(id, 5); // after "hello"
    expect(shape()).toEqual([["hello"], [" world"]]);
    const newId = doc.pages[0].roots[1];
    expect(editingId()).toBe(newId);
    expect(takeCaretFor(newId)).toBe(0);
  });

  it("keeps the hidden id:: on the original block when splitting (offset is in visible space)", () => {
    const dto = load([blk("hello world\nid:: 5462a76e-8aa4-4362-896e-9af769e5df77")]);
    const id = dto.blocks[0].id;
    splitBlock(id, 5); // caret after "hello" in the *visible* text
    // Original keeps "hello" + its id::; the new block gets just " world".
    expect(doc.byId[id].raw).toBe("hello\nid:: 5462a76e-8aa4-4362-896e-9af769e5df77");
    expect(doc.byId[doc.pages[0].roots[1]].raw).toBe(" world");
  });

  it("at start (offset 0), inserts an empty block before and the original keeps its uuid + content", () => {
    const dto = load([blk("a"), blk("query block")]);
    const id = dto.blocks[1].id;
    splitBlock(id, 0); // caret at head of "query block"
    expect(shape()).toEqual([["a"], [""], ["query block"]]);
    // the new empty block is being edited
    const emptyId = doc.pages[0].roots[1];
    expect(editingId()).toBe(emptyId);
    expect(emptyId).not.toBe(id);
    // crucially, the original uuid still holds the content (sidebar/refs stay valid)
    expect(doc.byId[id].raw).toBe("query block");
    expect(doc.pages[0].roots[2]).toBe(id);
  });

  it("at start, the original block keeps its children", () => {
    const dto = load([blk("q", [blk("child")])]);
    const id = dto.blocks[0].id;
    splitBlock(id, 0);
    expect(shape()).toEqual([[""], ["q", [["child"]]]]);
    expect(doc.byId[id].raw).toBe("q");
  });

  it("at end of an expanded parent, new block becomes first child", () => {
    const dto = load([blk("parent", [blk("child")])]);
    const id = dto.blocks[0].id;
    splitBlock(id, "parent".length);
    // new empty block is first child
    expect(shape()).toEqual([["parent", [[""], ["child"]]]]);
  });
});

describe("indent (Tab) — the Enter-then-Tab case", () => {
  it("makes a block the last child of its previous sibling, caret preserved", () => {
    const dto = load([blk("first"), blk("second")]);
    const second = dto.blocks[1].id;
    startEditing(second, 3);
    takeCaretFor(second); // consume initial
    indentBlock(second, 3);
    expect(shape()).toEqual([["first", [["second"]]]]);
    expect(editingId()).toBe(second);
    expect(takeCaretFor(second)).toBe(3); // caret kept at column 3
  });

  it("Enter then immediately Tab keeps editing the new block with caret", () => {
    const dto = load([blk("alpha")]);
    const id = dto.blocks[0].id;
    splitBlock(id, "alpha".length); // new sibling, empty, editing it
    const newId = editingId()!;
    takeCaretFor(newId);
    // user types nothing, presses Tab
    indentBlock(newId, 0);
    expect(shape()).toEqual([["alpha", [[""]]]]);
    expect(editingId()).toBe(newId);
    expect(takeCaretFor(newId)).toBe(0);
  });

  it("first child cannot indent (no previous sibling)", () => {
    const dto = load([blk("only")]);
    indentBlock(dto.blocks[0].id, 0);
    expect(shape()).toEqual([["only"]]);
  });
});

describe("outdent (Shift+Tab)", () => {
  it("moves block to be next sibling of its parent", () => {
    const dto = load([blk("parent", [blk("child")])]);
    const child = dto.blocks[0].children[0].id;
    outdentBlock(child, 2);
    expect(shape()).toEqual([["parent"], ["child"]]);
    expect(takeCaretFor(child)).toBe(2);
  });

  it("following siblings become children of the outdented block", () => {
    const dto = load([blk("p", [blk("a"), blk("b"), blk("c")])]);
    const a = dto.blocks[0].children[0].id;
    outdentBlock(a, 0);
    // a moves out after p, and b,c become a's children
    expect(shape()).toEqual([["p"], ["a", [["b"], ["c"]]]]);
  });
});

describe("merge (Backspace at 0)", () => {
  it("merges into previous visible block, caret at join point", () => {
    const dto = load([blk("foo"), blk("bar")]);
    const bar = dto.blocks[1].id;
    const ok = mergeWithPrev(bar);
    expect(ok).toBe(true);
    expect(shape()).toEqual([["foobar"]]);
    const foo = doc.pages[0].roots[0];
    expect(editingId()).toBe(foo);
    expect(takeCaretFor(foo)).toBe(3); // length of "foo"
  });

  it("reparents merged block's children", () => {
    const dto = load([blk("a"), blk("b", [blk("b1")])]);
    const b = dto.blocks[1].id;
    mergeWithPrev(b);
    expect(shape()).toEqual([["ab", [["b1"]]]]);
  });

  it("merges visible content, keeps prev's id::, drops the absorbed block's", () => {
    const dto = load([
      blk("foo\nid:: 11111111-1111-4111-8111-111111111111"),
      blk("bar\nid:: 22222222-2222-4222-8222-222222222222"),
    ]);
    const bar = dto.blocks[1].id;
    mergeWithPrev(bar);
    const foo = doc.pages[0].roots[0];
    // Visible content joined; only the previous block's id:: survives.
    expect(doc.byId[foo].raw).toBe("foobar\nid:: 11111111-1111-4111-8111-111111111111");
    expect(takeCaretFor(foo)).toBe(3); // join at end of "foo" (visible space)
  });

  it("first block can't merge backwards", () => {
    const dto = load([blk("only")]);
    expect(mergeWithPrev(dto.blocks[0].id)).toBe(false);
  });
});

describe("collapse / visible order", () => {
  it("collapsed blocks hide their children from visible order", () => {
    const dto = load([blk("p", [blk("c1"), blk("c2")]), blk("q")]);
    const p = dto.blocks[0].id;
    const order0 = visibleOrder().map((id) => doc.byId[id].raw);
    expect(order0).toEqual(["p", "c1", "c2", "q"]);
    toggleCollapse(p);
    const order1 = visibleOrder().map((id) => doc.byId[id].raw);
    expect(order1).toEqual(["p", "q"]);
  });
});

describe("undo / redo", () => {
  it("undoes a structural split and redoes it", () => {
    const dto = load([blk("hello world")]);
    const id = dto.blocks[0].id;
    splitBlock(id, 5);
    expect(shape()).toEqual([["hello"], [" world"]]);
    undo();
    expect(shape()).toEqual([["hello world"]]);
    redo();
    expect(shape()).toEqual([["hello"], [" world"]]);
  });

  it("coalesces typing in one block into a single undo step", () => {
    const dto = load([blk("a")]);
    const id = dto.blocks[0].id;
    setRaw(id, "ab");
    setRaw(id, "abc");
    setRaw(id, "abcd");
    undo(); // should revert to before this block's typing session ("a")
    expect(doc.byId[id].raw).toBe("a");
  });

  it("indent then undo restores the flat structure", () => {
    const dto = load([blk("first"), blk("second")]);
    indentBlock(dto.blocks[1].id, 0);
    expect(shape()).toEqual([["first", [["second"]]]]);
    undo();
    expect(shape()).toEqual([["first"], ["second"]]);
  });
});

describe("journals feed (multi-page)", () => {
  function feed() {
    loadFeed([
      { name: "Today", kind: "journal", title: "Today", pre_block: null, blocks: [blk("today a"), blk("today b")] },
      { name: "Yesterday", kind: "journal", title: "Yesterday", pre_block: null, blocks: [blk("yest a")] },
    ]);
  }

  it("visible order spans all pages in feed order", () => {
    feed();
    expect(visibleOrder().map((id) => doc.byId[id].raw)).toEqual([
      "today a",
      "today b",
      "yest a",
    ]);
  });

  it("does not merge a block into the previous page's block", () => {
    feed();
    const yestFirst = doc.pages[1].roots[0];
    // prevVisible(yestFirst) is "today b" on a different page — merge must no-op.
    expect(mergeWithPrev(yestFirst)).toBe(false);
    expect(doc.pages[1].roots.length).toBe(1);
  });

  it("splitting keeps the new block on the same page", () => {
    feed();
    const todayA = doc.pages[0].roots[0];
    splitBlock(todayA, "today a".length);
    const newId = editingId()!;
    expect(doc.byId[newId].page).toBe("Today");
    expect(doc.pages[0].roots.length).toBe(3);
  });
});
