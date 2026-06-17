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
  selectBlock,
  moveSelection,
  moveSelectionItems,
  moveBlockFeed,
  pageByName,
  carryUnfinished,
} from "./store";
import { journalTitle } from "./journal";
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

describe("move selection (mod+up/down in block-select)", () => {
  it("is a no-op at the top boundary (doesn't wrap the trailing blocks)", () => {
    const dto = load([blk("A"), blk("B"), blk("C")]);
    selectBlock(dto.blocks[0].id); // A
    moveSelection(1, true); // extend to B → selection [A, B]
    moveSelectionItems(-1); // up, but A is already at the top
    expect(shape()).toEqual([["A"], ["B"], ["C"]]);
  });

  it("moves a mid-list selection up by one", () => {
    const dto = load([blk("A"), blk("B"), blk("C")]);
    selectBlock(dto.blocks[1].id); // B
    moveSelection(1, true); // extend to C → selection [B, C]
    moveSelectionItems(-1);
    expect(shape()).toEqual([["B"], ["C"], ["A"]]);
  });
});

describe("cross-day move (journal feed as one list)", () => {
  const journal = (name: string, blocks: BlockDto[]): PageDto => ({
    name, kind: "journal", title: name, pre_block: null, blocks,
  });
  const raws = (name: string) => pageByName(name)!.roots.map((id) => doc.byId[id].raw);

  it("moves a root block up into the day above (feed order), keeping content", async () => {
    const today = journal("Today", [blk("t1")]);
    const older = journal("Older", [blk("o1"), blk("o2")]);
    loadFeed([today, older]); // today on top, older below
    const o1 = older.blocks[0].id;
    const res = await moveBlockFeed(o1, -1); // up → end of the day above
    expect(res).toBe("crossed");
    expect(raws("Today")).toEqual(["t1", "o1"]);
    expect(raws("Older")).toEqual(["o2"]);
    expect(doc.byId[o1].page).toBe("Today");
  });

  it("moves a root block down into the day below (prepended)", async () => {
    const today = journal("Today", [blk("t1"), blk("t2")]);
    const older = journal("Older", [blk("o1")]);
    loadFeed([today, older]);
    const t2 = today.blocks[1].id;
    const res = await moveBlockFeed(t2, 1); // down → start of the day below
    expect(res).toBe("crossed");
    expect(raws("Today")).toEqual(["t1"]);
    expect(raws("Older")).toEqual(["t2", "o1"]);
  });

  it("carries a block's subtree across with it", async () => {
    const today = journal("Today", [blk("t1")]);
    const older = journal("Older", [blk("o1", [blk("o1a")])]);
    loadFeed([today, older]);
    const o1 = older.blocks[0].id;
    const o1a = older.blocks[0].children[0].id;
    await moveBlockFeed(o1, -1);
    expect(doc.byId[o1].children.map((id) => doc.byId[id].raw)).toEqual(["o1a"]);
    expect(doc.byId[o1a].page).toBe("Today"); // subtree reassigned to the new day
  });

  it("can't move up past the top of the feed (today)", async () => {
    const today = journal("Today", [blk("t1")]);
    loadFeed([today]);
    const res = await moveBlockFeed(today.blocks[0].id, -1);
    expect(res).toBe("none");
    expect(raws("Today")).toEqual(["t1"]);
  });
});

describe("carry unfinished tasks → today", () => {
  const TODAY = journalTitle(new Date());
  const journal = (name: string, blocks: BlockDto[]): PageDto => ({
    name, kind: "journal", title: name, pre_block: null, blocks,
  });
  const raws = (name: string) => pageByName(name)!.roots.map((id) => doc.byId[id].raw);

  it("keepContext: moves whole top-level blocks containing an open task; leaves the rest", () => {
    const today = journal(TODAY, [blk("")]); // synthetic empty today
    const older = journal("Older", [
      blk("TODO A", [blk("DONE A1")]), // open task with done child → moves whole
      blk("DONE B"), // finished → stays
      blk("note C"), // plain note → stays
      blk("note D", [blk("TODO D1")]), // note containing an open task → moves whole (context)
    ]);
    loadFeed([today, older]);
    const moved = carryUnfinished(["Older"], true, null);
    expect(moved).toBe(2);
    expect(raws(TODAY)).toEqual(["TODO A", "note D"]); // empty placeholder dropped
    expect(raws("Older")).toEqual(["DONE B", "note C"]);
    // subtrees travel along
    const a = pageByName(TODAY)!.roots[0];
    expect(doc.byId[a].children.map((id) => doc.byId[id].raw)).toEqual(["DONE A1"]);
  });

  it("pull-out (keepContext off): extracts just the open-task subtrees, leaving scaffolding", () => {
    const today = journal(TODAY, [blk("existing")]);
    const older = journal("Older", [blk("note D", [blk("TODO D1", [blk("DONE D1a")])])]);
    loadFeed([today, older]);
    const moved = carryUnfinished(["Older"], false, null);
    expect(moved).toBe(1);
    expect(raws(TODAY)).toEqual(["existing", "TODO D1"]); // pulled out; note D stays
    expect(raws("Older")).toEqual(["note D"]);
    const t = pageByName(TODAY)!.roots[1];
    expect(doc.byId[t].children.map((id) => doc.byId[id].raw)).toEqual(["DONE D1a"]);
  });

  it("processes days in order (newest first ends up on top) and can add a header", () => {
    const today = journal(TODAY, [blk("")]);
    const d1 = journal("D1", [blk("TODO from-d1")]);
    const d2 = journal("D2", [blk("TODO from-d2")]);
    loadFeed([today, d1, d2]);
    carryUnfinished(["D1", "D2"], true, "Carried over");
    expect(raws(TODAY)).toEqual(["Carried over", "TODO from-d1", "TODO from-d2"]);
  });

  it("is a no-op when there are no open tasks", () => {
    const today = journal(TODAY, [blk("")]);
    const older = journal("Older", [blk("DONE x"), blk("just a note")]);
    loadFeed([today, older]);
    expect(carryUnfinished(["Older"], true, null)).toBe(0);
    expect(raws("Older")).toEqual(["DONE x", "just a note"]);
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
