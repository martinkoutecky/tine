// Headless tests for the editing tree ops + caret tracking — the M0 logic that
// the prior Qt attempt got wrong (caret lost on indent/split/merge). No DOM
// needed; these are pure operations on the store.

import { describe, it, expect, beforeEach } from "vitest";
import {
  page,
  loadPageDto,
  editingId,
  splitBlock,
  indentBlock,
  outdentBlock,
  mergeWithPrev,
  toggleCollapse,
  takeCaretFor,
  visibleOrder,
  startEditing,
} from "./store";
import type { BlockDto, PageDto } from "./types";

let counter = 0;
function blk(raw: string, children: BlockDto[] = []): BlockDto {
  return { id: `t${counter++}`, raw, collapsed: false, children };
}
function load(blocks: BlockDto[]): PageDto {
  const dto: PageDto = { name: "Test", kind: "page", title: "Test", pre_block: null, blocks };
  loadPageDto(dto);
  return dto;
}

/** Snapshot the tree as nested [raw, [children]] for easy assertions. */
function shape(ids: string[] = page.roots): any[] {
  return ids.map((id) => {
    const n = page.byId[id];
    return n.children.length ? [n.raw, shape(n.children)] : [n.raw];
  });
}

beforeEach(() => {
  counter = 0;
});

describe("split (Enter)", () => {
  it("splits a flat block into two siblings, caret at start of new", () => {
    const dto = load([blk("hello world")]);
    const id = dto.blocks[0].id;
    splitBlock(id, 5); // after "hello"
    expect(shape()).toEqual([["hello"], [" world"]]);
    const newId = page.roots[1];
    expect(editingId()).toBe(newId);
    expect(takeCaretFor(newId)).toBe(0);
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
    const foo = page.roots[0];
    expect(editingId()).toBe(foo);
    expect(takeCaretFor(foo)).toBe(3); // length of "foo"
  });

  it("reparents merged block's children", () => {
    const dto = load([blk("a"), blk("b", [blk("b1")])]);
    const b = dto.blocks[1].id;
    mergeWithPrev(b);
    expect(shape()).toEqual([["ab", [["b1"]]]]);
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
    const order0 = visibleOrder().map((id) => page.byId[id].raw);
    expect(order0).toEqual(["p", "c1", "c2", "q"]);
    toggleCollapse(p);
    const order1 = visibleOrder().map((id) => page.byId[id].raw);
    expect(order1).toEqual(["p", "q"]);
  });
});
