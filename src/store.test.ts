// Headless tests for the editing tree ops + caret tracking — the M0 logic that
// the prior Qt attempt got wrong (caret lost on indent/split/merge). No DOM
// needed; these are pure operations on the store.

import { describe, it, expect, beforeAll, beforeEach, afterEach, vi, type MockInstance } from "vitest";
import { initParser } from "./render/parse";
import {
  doc,
  resetStore,
  loadSingle,
  loadFeed,
  restoreTodayJournalInFeed,
  markDirty,
  flushPage,
  flushAll,
  forceSave,
  isDirty,
  deletePage,
  reloadDisposition,
  setBlockMoving,
  splitBlock,
  indentBlock,
  outdentBlock,
  mergeWithPrev,
  deleteBlock,
  toggleCollapse,
  visibleOrder,
  setRaw,
  undo,
  redo,
  selectBlock,
  moveSelection,
  moveSelectionItems,
  moveBlockFeed,
  moveBlock,
  indentSelection,
  reloadPage,
  forgetPage,
  pageByName,
  carryUnfinished,
  ensurePageLoaded,
  exportNodesFor,
  prevVisible,
  nextVisible,
  orderedListMarker,
  blockProperty,
  setBlockProperty,
  pageToDto,
  blockSubtreeMarkdown,
  selectionMarkdown,
  toggleListItemAtIndex,
  withUndoUnit,
} from "./store";
import { editingId, startEditing, takeCaretFor } from "./editorController";
import { exportOutline, DEFAULT_EXPORT_OPTIONS } from "./editor/exportText";
import { splitProps, joinProps, isBuiltinHidden, hideAll } from "./editor/properties";
import { setCopyIncludeSubtree, setCopyStripCollapsed } from "./copySettings";
import { backend, type Backend } from "./backend";
import {
  isConflicted,
  conflicts,
  clearConflict,
  favorites,
  recentPages,
  setFavorites,
  setRecentPages,
  seedFavorites,
} from "./ui";
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

beforeAll(() => initParser());

beforeEach(() => {
  counter = 0;
  resetStore();
  setFavorites([]);
  setRecentPages([]);
  setCopyIncludeSubtree(false); // copy prefs default OFF; reset so tests don't leak
  setCopyStripCollapsed(false);
});

describe("ordered list (logseq.order-list-type)", () => {
  const ORD = "logseq.order-list-type:: number";
  it("numbers the block itself across consecutive ordered siblings", () => {
    load([blk(`one\n${ORD}`), blk(`two\n${ORD}`), blk("plain"), blk(`three\n${ORD}`)]);
    const [a, b, c, d] = doc.pages[0].roots;
    expect(orderedListMarker(a)).toBe("1");
    expect(orderedListMarker(b)).toBe("2");
    expect(orderedListMarker(c)).toBe(null);
    expect(orderedListMarker(d)).toBe("1"); // the run restarts after the plain block
  });

  it("uses letters for a nested ordered list (ordered-ancestor depth 1)", () => {
    load([blk(`parent\n${ORD}`, [blk(`child\n${ORD}`)])]);
    const parent = doc.pages[0].roots[0];
    const child = doc.byId[parent].children[0];
    expect(orderedListMarker(parent)).toBe("1");
    expect(orderedListMarker(child)).toBe("a");
  });

  it("Enter on an ordered item makes the new sibling ordered too", () => {
    const dto = load([blk(`item\n${ORD}`)]);
    splitBlock(dto.blocks[0].id, 4); // caret after "item"
    const newId = doc.pages[0].roots[1];
    expect(blockProperty(newId, "logseq.order-list-type")).toBe("number");
    expect(orderedListMarker(newId)).toBe("2");
  });
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

describe("exportNodesFor (Copy / export selection)", () => {
  it("a parent + its selected descendants export ONCE (no child duplication)", () => {
    // selectedIds() is a flat slice of visible order, so a parent selection
    // includes its children; exporting must not emit them twice.
    const parent = blk("parent", [blk("c1"), blk("c2"), blk("c3")]);
    load([parent]);
    const ids = [parent.id, parent.children[0].id, parent.children[1].id, parent.children[2].id];
    const nodes = exportNodesFor(ids);
    expect(nodes.length).toBe(1); // only the parent root
    expect(exportOutline(nodes, { ...DEFAULT_EXPORT_OPTIONS, content: "source" })).toBe("- parent\n\t- c1\n\t- c2\n\t- c3");
  });

  it("sibling roots both export (no false dedup)", () => {
    const a = blk("a");
    const b = blk("b");
    load([a, b]);
    expect(exportOutline(exportNodesFor([a.id, b.id]), { ...DEFAULT_EXPORT_OPTIONS, content: "source" })).toBe("- a\n- b");
  });
});

describe("clipboard copy strips id:: (OG parity)", () => {
  it("blockSubtreeMarkdown(stripId) drops id:: but keeps content + collapsed::", () => {
    // OG's copy-to-clipboard-without-id-property! strips only id::, not collapsed::.
    const b = blk("referenced block\ncollapsed:: true\nid:: 5462a76e-8aa4-4362-896e-9af769e5df77");
    load([b]);
    const copied = blockSubtreeMarkdown(b.id, 0, true);
    expect(copied).not.toContain("id::");
    expect(copied).toContain("referenced block");
    expect(copied).toContain("collapsed:: true");
  });

  it("default (no strip) keeps id:: — e.g. quick-capture writing to a journal file", () => {
    const b = blk("note\nid:: 5462a76e-8aa4-4362-896e-9af769e5df77");
    load([b]);
    expect(blockSubtreeMarkdown(b.id)).toContain("id:: 5462a76e-8aa4-4362-896e-9af769e5df77");
  });

  it("id:: inside a code fence is NOT stripped (fence-aware)", () => {
    const b = blk("```\nid:: literal-in-code\n```\nid:: 5462a76e-8aa4-4362-896e-9af769e5df77");
    load([b]);
    const copied = blockSubtreeMarkdown(b.id, 0, true);
    expect(copied).toContain("id:: literal-in-code"); // fenced content survives
    expect(copied).not.toContain("id:: 5462a76e"); // the real property is gone
  });

  it("selectionMarkdown: default copies ONLY the selected block (not unselected children)", () => {
    setCopyIncludeSubtree(false); // Tine default
    const parent = blk("parent\nid:: 11111111-1111-1111-1111-111111111111", [
      blk("child\nid:: 22222222-2222-2222-2222-222222222222"),
    ]);
    load([parent]);
    selectBlock(parent.id); // only the parent
    const md = selectionMarkdown();
    expect(md).not.toContain("id::");
    expect(md).toContain("- parent");
    expect(md).not.toContain("child"); // child wasn't selected → excluded
  });

  it("selectionMarkdown: include-subtree mode (OG) copies the whole sub-tree", () => {
    setCopyIncludeSubtree(true); // = Logseq behavior
    const parent = blk("parent", [blk("child")]);
    load([parent]);
    selectBlock(parent.id);
    const md = selectionMarkdown();
    expect(md).toContain("- parent");
    expect(md).toContain("\t- child");
    setCopyIncludeSubtree(false); // restore default for other tests
  });

  it("collapsed:: kept by default (OG), stripped when the option is on", () => {
    const b = blk("note\ncollapsed:: true\nid:: 33333333-3333-3333-3333-333333333333");
    load([b]);
    expect(blockSubtreeMarkdown(b.id, 0, true)).toContain("collapsed:: true");
    expect(blockSubtreeMarkdown(b.id, 0, true, true)).not.toContain("collapsed::");
  });
});

describe("property splitting is fence-aware", () => {
  it("keeps built-in id::/collapsed:: inside a code fence as visible content", () => {
    const raw = "```text\nid:: literal-in-code\ncollapsed:: true\n```\nid:: real-block-id";
    const { visible, hidden } = splitProps(raw, isBuiltinHidden);
    expect(hidden).toBe("id:: real-block-id"); // only the real trailing property is hidden
    expect(visible).toContain("id:: literal-in-code"); // fenced lines stay put
    expect(visible).toContain("collapsed:: true");
    // Round-trip (focus→blur) must reconstruct the identical raw, not corrupt it.
    expect(joinProps(visible, hidden)).toBe(raw);
  });

  it("joinProps on a metadata-only block adds no leading blank line", () => {
    // splitProps of "id:: x" → visible "", hidden "id:: x"; reassembly must not
    // prepend a newline (that would corrupt the block with a blank first line).
    const { visible, hidden } = splitProps("id:: x", isBuiltinHidden);
    expect(visible).toBe("");
    expect(hidden).toBe("id:: x");
    expect(joinProps(visible, hidden)).toBe("id:: x");
  });

  it("hideAll (annotation blocks) is ALSO fence-aware — a key:: inside a fence stays", () => {
    // The old annotation splitter wasn't fence-aware and would yank this out.
    const raw = "highlight text\n```\nkey:: not-a-prop\n```\nls-type:: annotation\nhl-page:: 3";
    const { visible, hidden } = splitProps(raw, hideAll);
    expect(visible).toBe("highlight text\n```\nkey:: not-a-prop\n```");
    expect(hidden).toBe("ls-type:: annotation\nhl-page:: 3");
    expect(joinProps(visible, hidden)).toBe(raw);
  });
});

describe("trailing space: kept in the editor buffer, trimmed only on save", () => {
  // Regression for the "backspace eats the preceding space" bug: the live buffer
  // (doc.byId[].raw — what the reactive textarea mirrors) must KEEP a trailing
  // space the user left, so deleting a word back to "this is a " doesn't pull the
  // space out from under the caret. OG trims on save, not on every keystroke — so
  // the disk DTO (pageToDto) is the only place the space is dropped.
  it("setRaw keeps a trailing space in the buffer; pageToDto trims it", () => {
    const dto = load([blk("this is a test")]);
    const id = dto.blocks[0].id;
    setRaw(id, "this is a "); // backspaced "test" away, trailing space remains
    expect(doc.byId[id].raw).toBe("this is a "); // buffer keeps the space
    const out = pageToDto("Test")!;
    expect(out.blocks[0].raw).toBe("this is a"); // disk DTO trims it
  });

  it("a block with nothing to trim serializes byte-identically (no churn)", () => {
    load([blk("plain"), blk("with id\nid:: 5462a76e-8aa4-4362-896e-9af769e5df77")]);
    const out = pageToDto("Test")!;
    expect(out.blocks[0].raw).toBe("plain");
    expect(out.blocks[1].raw).toBe("with id\nid:: 5462a76e-8aa4-4362-896e-9af769e5df77");
  });
});

describe("undo history is graph-local", () => {
  it("resetStore clears undo so an undo can't restore the previous graph", () => {
    load([blk("hello world")]);
    splitBlock(doc.pages[0].roots[0], 5); // structural op → undo entry exists
    expect(doc.pages[0].roots.length).toBe(2);

    resetStore(); // simulate a graph switch
    load([blk("fresh graph")]); // a different graph's page
    const before = JSON.stringify(doc.pages[0].roots);
    undo(); // must be a no-op — the old graph's snapshot is gone
    expect(JSON.stringify(doc.pages[0].roots)).toBe(before);
    expect(doc.byId[doc.pages[0].roots[0]].raw).toBe("fresh graph");
  });
});

describe("cross-page duplicate id::", () => {
  const page = (name: string, blocks: BlockDto[]): PageDto => ({
    name, kind: "page", title: name, pre_block: null, blocks,
  });

  it("re-keys a duplicate id:: on a second page so the two blocks stay distinct", () => {
    // Two files carrying the SAME persisted id (e.g. copy-pasted raw, or a sync
    // hiccup) — the global byId must not collapse them into one node.
    ensurePageLoaded(page("A", [{ id: "dup", raw: "alpha\nid:: dup", collapsed: false, children: [] }]));
    ensurePageLoaded(page("B", [{ id: "dup", raw: "beta\nid:: dup", collapsed: false, children: [] }]));

    const aRoot = pageByName("A")!.roots[0];
    const bRoot = pageByName("B")!.roots[0];
    expect(aRoot).toBe("dup"); // first page keeps the id
    expect(bRoot).not.toBe("dup"); // second page re-keyed
    expect(doc.byId[aRoot].raw).toContain("alpha");
    expect(doc.byId[bRoot].raw).toContain("beta");

    // Editing B's block must not touch A's (the corruption the guard prevents).
    setRaw(bRoot, "beta edited\nid:: dup");
    expect(doc.byId[aRoot].raw).toContain("alpha");
    expect(doc.byId[aRoot].page).toBe("A");
    expect(doc.byId[bRoot].page).toBe("B");
  });
});

describe("reloadDisposition (watcher reload guard)", () => {
  const j = (name: string, blocks: BlockDto[]): PageDto => ({
    name, kind: "journal", title: name, pre_block: null, blocks,
  });
  it("reload when clean; skip while editing a block on it or mid block-move", () => {
    loadFeed([j("Today", [blk("t1")])]);
    expect(reloadDisposition("Today")).toBe("reload");
    setBlockMoving(true);
    expect(reloadDisposition("Today")).toBe("skip"); // a move is mid-flight
    setBlockMoving(false);
    expect(reloadDisposition("Today")).toBe("reload");
    startEditing(pageByName("Today")!.roots[0], 0, null);
    expect(reloadDisposition("Today")).toBe("skip"); // a block on it is focused
  });
  it("conflict when the page has unsaved edits (never clobber)", () => {
    loadFeed([j("D", [blk("d1")])]);
    markDirty("D");
    expect(reloadDisposition("D")).toBe("conflict");
  });
});

describe("page-scoped structural undo", () => {
  const journal = (name: string, blocks: BlockDto[]): PageDto => ({
    name, kind: "journal", title: name, pre_block: null, blocks,
  });
  const raws = (name: string) => pageByName(name)!.roots.map((id) => doc.byId[id].raw);

  it("undo of a single-page edit restores that page and leaves other loaded pages untouched", () => {
    const today = journal("Today", [blk("t1")]);
    const older = journal("Older", [blk("o1"), blk("o2")]);
    loadFeed([today, older]);
    const olderIds = pageByName("Older")!.roots.slice();

    splitBlock(today.blocks[0].id, 1); // edit ONLY Today: "t1" -> "t","1"
    expect(raws("Today")).toEqual(["t", "1"]);

    undo();
    expect(raws("Today")).toEqual(["t1"]); // restored
    // Other page's nodes are completely unaffected (same ids, same content).
    expect(pageByName("Older")!.roots).toEqual(olderIds);
    expect(raws("Older")).toEqual(["o1", "o2"]);

    redo();
    expect(raws("Today")).toEqual(["t", "1"]);
    expect(raws("Older")).toEqual(["o1", "o2"]);
  });

  it("undo preserves a path-pinned page's `path` (a #21 stray must not misroute its save)", () => {
    // `path` pins the save to the exact file the page came from (a duplicate-day
    // stray). The undo clone used to drop it, so undoing an edit re-routed the
    // next save to the CANONICAL file. Snapshot → edit → undo must keep `path`.
    const stray: PageDto = {
      name: "Today", kind: "journal", title: "Today", pre_block: null,
      blocks: [blk("t1")], path: "journals/Friday, 26-06-2026.md",
    };
    loadFeed([stray]);
    expect(pageByName("Today")!.path).toBe("journals/Friday, 26-06-2026.md");
    splitBlock(stray.blocks[0].id, 1); // structural op → snapshots this page
    undo();
    expect(pageByName("Today")!.path).toBe("journals/Friday, 26-06-2026.md");
    redo();
    expect(pageByName("Today")!.path).toBe("journals/Friday, 26-06-2026.md");
  });

  it("undo removes an op-added node from byId entirely (root-walk purge, no leak)", () => {
    const today = journal("Today", [blk("t1")]);
    loadFeed([today]);
    splitBlock(today.blocks[0].id, 1); // adds a new node on Today
    const addedId = pageByName("Today")!.roots[1];
    expect(doc.byId[addedId]).toBeTruthy();

    undo();
    // The scoped restore must purge the affected page's current subtree (incl. the
    // op-added node) by walking roots — not leave it dangling in byId.
    expect(doc.byId[addedId]).toBeUndefined();
    expect(raws("Today")).toEqual(["t1"]);
  });

  it("cross-day move undo leaves an unrelated loaded page intact", async () => {
    const today = journal("Today", [blk("t1")]);
    const older = journal("Older", [blk("o1")]);
    loadFeed([today, older]);
    // A separate page in the working set (e.g. open in the sidebar), loaded after
    // the move's snapshot would be taken.
    ensurePageLoaded({ name: "Side", kind: "page", title: "Side", pre_block: null, blocks: [blk("s1")] });
    const sideId = pageByName("Side")!.roots[0];

    await moveBlockFeed(older.blocks[0].id, -1); // cross-day move (scoped to Today+Older)
    undo();

    expect(raws("Today")).toEqual(["t1"]);
    expect(raws("Older")).toEqual(["o1"]);
    // The unrelated page must survive the undo (the old null-fallback wiped it).
    expect(pageByName("Side")).toBeTruthy();
    expect(doc.byId[sideId]?.raw).toBe("s1");
  });

  it("undo of a cross-page move restores both pages (full-snapshot fallback)", async () => {
    const today = journal("Today", [blk("t1")]);
    const older = journal("Older", [blk("o1"), blk("o2")]);
    loadFeed([today, older]);
    const o1 = older.blocks[0].id;
    await moveBlockFeed(o1, -1); // o1 crosses up into Today
    expect(raws("Today")).toEqual(["t1", "o1"]);
    expect(raws("Older")).toEqual(["o2"]);

    undo();
    expect(raws("Today")).toEqual(["t1"]);
    expect(raws("Older")).toEqual(["o1", "o2"]);
    expect(doc.byId[o1].page).toBe("Older"); // page ownership restored too
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

  it("removes the carried tasks and leaves finished tasks AND blank spacer bullets untouched", () => {
    const today = journal(TODAY, [blk("")]);
    // The reported case: open tasks interleaved with a finished task and a blank
    // spacer bullet. Carrying must remove ONLY the open tasks; the spacer stays.
    const older = journal("Older", [
      blk("TODO bla"),
      blk("TODO ble"),
      blk("TODO something"),
      blk("DONE something else"),
      blk(""), // intentional spacer — must survive the carry
      blk("DONE another thing"),
    ]);
    loadFeed([today, older]);
    expect(carryUnfinished(["Older"], false, null)).toBe(3);
    expect(raws("Older")).toEqual(["DONE something else", "", "DONE another thing"]);
  });

  it("leaves a blank parent that only held a carried task (no-task blocks are never touched)", () => {
    const today = journal(TODAY, [blk("")]);
    const older = journal("Older", [blk("", [blk("TODO a")])]);
    loadFeed([today, older]);
    carryUnfinished(["Older"], false, null);
    expect(raws("Older")).toEqual([""]); // the empty parent stays — it had no task marker
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

  it("supports deleting a visually empty leading block and focusing the next block", () => {
    const dto = load([
      blk("id:: 11111111-1111-4111-8111-111111111111"),
      blk("next"),
    ]);
    const first = dto.blocks[0].id;
    const next = nextVisible(first);

    expect(mergeWithPrev(first)).toBe(false);
    expect(splitProps(doc.byId[first].raw, isBuiltinHidden).visible.trim()).toBe("");
    expect(doc.byId[first].children).toHaveLength(0);
    expect(next).toBe(dto.blocks[1].id);

    deleteBlock(first);
    startEditing(next!, 0);

    expect(shape()).toEqual([["next"]]);
    expect(editingId()).toBe(next);
    expect(takeCaretFor(next!)).toBe(0);
  });

  // The quick-capture window edits a scratch page that's never part of the main
  // routed view (mainPages()), so visibleData() doesn't index its blocks.
  // prevVisible/nextVisible must fall back to the block's own page — otherwise
  // Backspace-merge and Up/Down nav are dead in the capture window.
  it("merges + navigates on a detached page absent from the main view", () => {
    ensurePageLoaded({
      name: "·capture·",
      kind: "page",
      title: "·capture·",
      pre_block: null,
      blocks: [blk("first"), blk("second")],
    });
    const cap = pageByName("·capture·")!;
    const [first, second] = cap.roots;
    expect(prevVisible(second)).toBe(first);
    expect(nextVisible(first)).toBe(second);
    expect(prevVisible(first)).toBe(null);
    expect(mergeWithPrev(second)).toBe(true);
    const after = pageByName("·capture·")!;
    expect(after.roots.length).toBe(1);
    expect(doc.byId[after.roots[0]].raw).toBe("firstsecond");
  });
});

describe("collapse / visible order", () => {
  it("collapsed blocks hide their children, and persist collapsed:: in raw", () => {
    const dto = load([blk("p", [blk("c1"), blk("c2")]), blk("q")]);
    const p = dto.blocks[0].id;
    const order0 = visibleOrder().map((id) => doc.byId[id].raw.split("\n")[0]);
    expect(order0).toEqual(["p", "c1", "c2", "q"]);
    toggleCollapse(p);
    const order1 = visibleOrder().map((id) => doc.byId[id].raw.split("\n")[0]);
    expect(order1).toEqual(["p", "q"]);
    // Collapse is mirrored into the block's raw so it survives a reload (OG stores
    // it as a block property); expanding again removes the line.
    expect(doc.byId[p].raw).toContain("collapsed:: true");
    toggleCollapse(p);
    expect(doc.byId[p].raw).not.toContain("collapsed::");
  });

  it("treats grid blocks as opaque in visible order", () => {
    const grid = blk("grid\ntine.view:: grid", [
      blk("", [blk("r1c1"), blk("r1c2")]),
      blk("", [blk("r2c1")]),
    ]);
    grid.properties = [["tine.view", "grid"]];
    const plain = blk("plain", [blk("plain child")]);
    const dto = load([grid, plain]);

    expect(visibleOrder().map((id) => doc.byId[id].raw.split("\n")[0])).toEqual([
      "grid",
      "plain",
      "plain child",
    ]);

    toggleCollapse(dto.blocks[0].id);
    expect(visibleOrder().map((id) => doc.byId[id].raw.split("\n")[0])).toEqual([
      "grid",
      "plain",
      "plain child",
    ]);

    toggleCollapse(dto.blocks[1].id);
    expect(visibleOrder().map((id) => doc.byId[id].raw.split("\n")[0])).toEqual(["grid", "plain"]);
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

  it("withUndoUnit coalesces multiple raw edits into one undo step", () => {
    const dto = load([blk("one"), blk("two")]);
    const [a, b] = dto.blocks;

    withUndoUnit("composite", ["Test"], () => {
      setRaw(a.id, "ONE");
      setRaw(b.id, "TWO");
    });

    expect(doc.byId[a.id].raw).toBe("ONE");
    expect(doc.byId[b.id].raw).toBe("TWO");

    undo();
    expect(doc.byId[a.id].raw).toBe("one");
    expect(doc.byId[b.id].raw).toBe("two");
  });

  it("withUndoUnit rolls back a throwing composite and leaves no undo entry", () => {
    const dto = load([blk("one"), blk("two")]);
    const [a, b] = dto.blocks;

    expect(() =>
      withUndoUnit("throwing", ["Test"], () => {
        setRaw(a.id, "ONE");
        setRaw(b.id, "TWO");
        throw new Error("boom");
      })
    ).toThrow("boom");

    expect(doc.byId[a.id].raw).toBe("one");
    expect(doc.byId[b.id].raw).toBe("two");
    undo();
    expect(doc.byId[a.id].raw).toBe("one");
    expect(doc.byId[b.id].raw).toBe("two");
  });

  it("withUndoUnit redo works after undo", () => {
    const dto = load([blk("one"), blk("two")]);
    const [a, b] = dto.blocks;

    withUndoUnit("composite", ["Test"], () => {
      setRaw(a.id, "ONE");
      setRaw(b.id, "TWO");
    });

    undo();
    redo();
    expect(doc.byId[a.id].raw).toBe("ONE");
    expect(doc.byId[b.id].raw).toBe("TWO");
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

describe("stale undo is dropped on external reload / forget (ds8-1)", () => {
  const page = (name: string, blocks: BlockDto[]): PageDto => ({
    name, kind: "page", title: name, pre_block: null, blocks,
  });

  it("undo after an external reload can't clobber the reloaded content", () => {
    loadSingle(page("P", [blk("original")]));
    splitBlock(doc.pages[0].roots[0], 4); // structural op → undo entry for P exists
    expect(doc.pages[0].roots.length).toBe(2);
    // External edit lands on disk; we reload P with new content + rev.
    reloadPage({
      name: "P", kind: "page", title: "P", pre_block: null, rev: "r2",
      blocks: [{ id: "x", raw: "external version", collapsed: false, children: [] }],
    });
    const before = doc.pages[0].roots.map((id) => doc.byId[id].raw);
    undo(); // must be a no-op — the pre-reload snapshot was invalidated
    expect(doc.pages[0].roots.map((id) => doc.byId[id].raw)).toEqual(before);
    expect(doc.byId[doc.pages[0].roots[0]].raw).toBe("external version");
  });

  it("undo after forgetPage can't resurrect the page", () => {
    loadSingle(page("P", [blk("a")]));
    splitBlock(doc.pages[0].roots[0], 1);
    forgetPage("P"); // e.g. accepting "use disk version" after an external delete
    expect(pageByName("P")).toBeUndefined();
    undo(); // must NOT re-add P (which would recreate the deleted file)
    expect(pageByName("P")).toBeUndefined();
  });
});

describe("root-to-root drop across pages targets the drop page (#38)", () => {
  const journal = (name: string, blocks: BlockDto[]): PageDto => ({
    name, kind: "journal", title: name, pre_block: null, blocks,
  });
  const raws = (name: string) => pageByName(name)!.roots.map((id) => doc.byId[id].raw);

  it("a root block dropped onto another day's root lands on that day, not the source", async () => {
    const today = journal("Today", [blk("t1")]);
    const older = journal("Older", [blk("o1")]);
    loadFeed([today, older]);
    const t1 = today.blocks[0].id;
    // Drop t1 (a root) after o1 (a root on Older): newParent=null, targetPage=Older.
    await moveBlock(t1, null, 1, "Older");
    expect(raws("Today")).toEqual([]); // left the source page
    expect(raws("Older")).toEqual(["o1", "t1"]); // landed on the drop page
    expect(doc.byId[t1].page).toBe("Older");
  });
});

describe("selection indent is single-page (ds8-2)", () => {
  const journal = (name: string, blocks: BlockDto[]): PageDto => ({
    name, kind: "journal", title: name, pre_block: null, blocks,
  });

  it("indenting a cross-day selection leaves the other day's block in place", () => {
    const today = journal("Today", [blk("t1"), blk("t2")]);
    const older = journal("Older", [blk("o1")]);
    loadFeed([today, older]);
    const t1 = today.blocks[0].id, t2 = today.blocks[1].id, o1 = older.blocks[0].id;
    selectBlock(t2); // anchor on Today
    moveSelection(1, true); // extend down across the day boundary to o1
    indentSelection();
    // t2 indents under t1 (same page); o1 must NOT have been dragged onto Today.
    expect(doc.byId[o1].page).toBe("Older");
    expect(pageByName("Older")!.roots).toContain(o1);
    expect(doc.byId[t2].parent).toBe(t1);
    expect(doc.byId[t2].page).toBe("Today");
  });
});

// Characterization tests for the debounced persistence engine (markDirty →
// scheduleSave/doSave/flushPage/flushAll/forceSave + the dirty/baseRev/
// deletedPages/conflict guards). These pin the save behaviour so the R2
// extraction into a SaveCoordinator is provably behaviour-preserving.
describe("save engine (persistence)", () => {
  let saveSpy: MockInstance<Backend["savePage"]>;
  beforeEach(() => {
    conflicts()
      .slice()
      .forEach(clearConflict); // ui conflicts aren't cleared by resetStore
    vi.useFakeTimers();
    saveSpy = vi.spyOn(backend(), "savePage").mockResolvedValue("rev1");
  });
  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
    saveSpy.mockRestore();
  });

  it("debounces dirty pages into one batched save", async () => {
    load([blk("hello")]);
    markDirty("Test");
    markDirty("Test"); // coalesced into the same 400ms batch
    expect(saveSpy).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(400);
    expect(saveSpy).toHaveBeenCalledTimes(1);
    expect((saveSpy.mock.calls[0][0] as { name: string }).name).toBe("Test");
    expect(isDirty("Test")).toBe(false);
  });

  it("flushPage writes immediately and advances the baseline rev", async () => {
    load([blk("x")]);
    saveSpy.mockResolvedValue("rev2");
    markDirty("Test");
    expect(await flushPage("Test")).toBe(true);
    expect(saveSpy).toHaveBeenCalledTimes(1);
    // Next save sends the rev returned by the previous one as its baseRev.
    markDirty("Test");
    await flushPage("Test");
    expect(saveSpy.mock.calls[1][1]).toBe("rev2");
  });

  it("a conflict marks the page (no clobber) and flushAll reports failure", async () => {
    load([blk("x")]);
    markDirty("Test");
    saveSpy.mockRejectedValueOnce(new Error("conflict"));
    expect(await flushAll()).toBe(false);
    expect(isConflicted("Test")).toBe(true);
  });

  it("a transient error keeps the page dirty for retry", async () => {
    load([blk("x")]);
    markDirty("Test");
    saveSpy.mockRejectedValueOnce(new Error("disk full"));
    expect(await flushPage("Test")).toBe(false);
    expect(isDirty("Test")).toBe(true);
    expect(await flushPage("Test")).toBe(true); // retry succeeds
  });

  it("a tombstoned (deleted) page is never written", async () => {
    load([blk("x")]);
    markDirty("Test");
    await deletePage("Test", "page"); // tombstones the page
    saveSpy.mockClear();
    markDirty("Test"); // a stray queued save after delete must not recreate it
    expect(await flushPage("Test")).toBe(true);
    expect(saveSpy).not.toHaveBeenCalled();
  });

  it("deletePage removes journal feed entries plus sidebar favorites and recents", async () => {
    loadFeed([
      { name: "Today", kind: "journal", title: "Today", pre_block: null, blocks: [blk("today")] },
      { name: "Older", kind: "journal", title: "Older", pre_block: null, blocks: [blk("older")] },
    ]);
    setFavorites([{ name: "Older", kind: "journal" }, { name: "Pinned", kind: "page" }]);
    setRecentPages([{ name: "Older", kind: "journal" }, { name: "Pinned", kind: "page" }]);

    expect(await deletePage("Older", "journal")).toBe(true);

    expect(doc.feed).toEqual(["Today"]);
    expect(pageByName("Older")).toBeUndefined();
    expect(favorites()).toEqual([{ name: "Pinned", kind: "page" }]);
    expect(recentPages()).toEqual([{ name: "Pinned", kind: "page" }]);
  });

  it("seedFavorites replaces (per-graph) on graph open, clearing to empty for a graph with none", () => {
    // Graph A has favorites.
    seedFavorites(["Inbox", "2026-07-05"]);
    expect(favorites()).toEqual([
      { name: "Inbox", kind: "page" },
      // A journal-titled favorite is re-derived as a journal so it routes correctly.
      { name: "2026-07-05", kind: "journal" },
    ]);
    // Switch to graph B, which has NO favorites — must not linger from graph A.
    seedFavorites([]);
    expect(favorites()).toEqual([]);
    // Switch to graph C with a different set — full replace, not a merge.
    seedFavorites(["Reading List"]);
    expect(favorites()).toEqual([{ name: "Reading List", kind: "page" }]);
  });

  it("restores today's empty journal at the top of the feed after deleting today in place (#17)", async () => {
    const today = journalTitle(new Date());
    loadFeed([
      { name: today, kind: "journal", title: today, pre_block: null, blocks: [blk("today content")] },
      { name: "Older", kind: "journal", title: "Older", pre_block: null, blocks: [blk("older")] },
    ]);

    expect(await deletePage(today, "journal")).toBe(true);
    expect(doc.feed).toEqual(["Older"]); // deletePage alone drops today from the feed
    restoreTodayJournalInFeed(); // ContextMenu re-runs this on the journals feed

    expect(doc.feed).toEqual([today, "Older"]); // today back on top…
    const page = pageByName(today)!;
    expect(page.roots).toHaveLength(1); // …as a single empty editable block
    expect(doc.byId[page.roots[0]].raw).toBe("");

    // The placeholder is writable: the delete tombstone was lifted, so the first
    // edit saves a fresh file (not silently swallowed like a still-deleted page).
    saveSpy.mockClear();
    markDirty(today);
    expect(await flushPage(today)).toBe(true);
    expect(saveSpy).toHaveBeenCalledTimes(1);
    expect((saveSpy.mock.calls[0][0] as { name: string }).name).toBe(today);
  });

  it("keeps today untouched when an OLDER day is deleted from the feed (#17 no-op)", async () => {
    const today = journalTitle(new Date());
    loadFeed([
      { name: today, kind: "journal", title: today, pre_block: null, blocks: [blk("today content")] },
      { name: "Older", kind: "journal", title: "Older", pre_block: null, blocks: [blk("older")] },
    ]);

    expect(await deletePage("Older", "journal")).toBe(true);
    restoreTodayJournalInFeed(); // called on every journals-feed delete; must not disturb today

    expect(doc.feed).toEqual([today]); // today's real content still there, not replaced
    expect(doc.byId[pageByName(today)!.roots[0]].raw).toBe("today content");
  });

  it("forceSave overwrites even a conflicted page (force=true)", async () => {
    load([blk("x")]);
    markDirty("Test");
    saveSpy.mockRejectedValueOnce(new Error("conflict"));
    await flushPage("Test");
    expect(isConflicted("Test")).toBe(true);
    saveSpy.mockResolvedValue("rev3");
    expect(await forceSave("Test")).toBe(true);
    expect(saveSpy.mock.calls.at(-1)![2]).toBe(true); // force flag
  });
});

describe("undo survives a self-write reload echo (Ctrl+Z of a delete)", () => {
  const echo = (blocks: BlockDto[]): PageDto => ({
    name: "Test",
    kind: "page",
    title: "Test",
    pre_block: null,
    blocks,
  });

  it("keeps the delete-undo entry when a reload's content matches memory", () => {
    load([blk("keep"), blk("victim")]);
    deleteBlock(doc.pages[0].roots[1]);
    expect(shape()).toEqual([["keep"]]);
    // The watcher re-reports our OWN just-saved content (identical) — this must NOT
    // drop the undo entry we pushed for the delete.
    reloadPage(echo([{ id: "x", raw: "keep", collapsed: false, children: [] }]));
    undo();
    expect(shape()).toEqual([["keep"], ["victim"]]); // deletion undone
  });

  it("still invalidates undo on a GENUINE external change", () => {
    load([blk("keep"), blk("victim")]);
    deleteBlock(doc.pages[0].roots[1]);
    // Different content on disk → a real external edit → undo is (correctly) dropped.
    reloadPage(echo([{ id: "x", raw: "changed elsewhere", collapsed: false, children: [] }]));
    undo();
    expect(shape()).toEqual([["changed elsewhere"]]); // undo was a no-op; external content kept
  });
});

// Audit fix C3 (data-safety): the AST list checkbox toggle targets the source line
// by document POSITION, so two items with the same label flip independently — the
// old text-match toggled the first matching raw line regardless of which was clicked.
describe("toggleListItemAtIndex (positional checkbox toggle)", () => {
  it("flips the exact line among identical checkbox labels", () => {
    const b = blk("Title\n+ [ ] same\n+ [ ] same");
    load([b]);
    toggleListItemAtIndex(b.id, 2); // line index 2 = the SECOND "+ [ ] same"
    expect(doc.byId[b.id].raw).toBe("Title\n+ [ ] same\n+ [x] same");
  });

  it("ignores a non-checkbox line index (no-op, no corruption)", () => {
    const b = blk("Title\n+ [ ] a");
    load([b]);
    toggleListItemAtIndex(b.id, 0); // "Title" is not a checkbox line
    expect(doc.byId[b.id].raw).toBe("Title\n+ [ ] a");
  });
});

describe("setBlockProperty placement & fence safety", () => {
  it("inserts after the first line, before body text", () => {
    const b = blk("Title\nbody line");
    load([b]);
    setBlockProperty(b.id, "tine.view", "grid");
    expect(doc.byId[b.id].raw).toBe("Title\ntine.view:: grid\nbody line");
  });

  it("keeps planning lines before properties (OG order)", () => {
    const b = blk("TODO task\nSCHEDULED: <2026-07-10 Fri>\nbody");
    load([b]);
    setBlockProperty(b.id, "effort", "2h");
    expect(doc.byId[b.id].raw).toBe("TODO task\nSCHEDULED: <2026-07-10 Fri>\neffort:: 2h\nbody");
  });

  it("replaces an existing head property in place", () => {
    const b = blk("Title\na:: 1\nb:: 2\nbody");
    load([b]);
    setBlockProperty(b.id, "a", "9");
    expect(doc.byId[b.id].raw).toBe("Title\nb:: 2\na:: 9\nbody");
  });

  it("NEVER touches property-looking lines inside a code fence", () => {
    const raw = "Title\n```\nfence:: not-a-prop\n```\ntail";
    const b = blk(raw);
    load([b]);
    setBlockProperty(b.id, "x", "1");
    expect(doc.byId[b.id].raw).toBe("Title\nx:: 1\n```\nfence:: not-a-prop\n```\ntail");
    setBlockProperty(b.id, "fence", "rewrite");
    // the fenced line is untouched; the new property lands in the head
    expect(doc.byId[b.id].raw).toBe(
      "Title\nx:: 1\nfence:: rewrite\n```\nfence:: not-a-prop\n```\ntail"
    );
  });

  it("removes a legacy trailing property without disturbing the body", () => {
    const b = blk("Title\nbody\nold:: legacy");
    load([b]);
    setBlockProperty(b.id, "old", "new");
    expect(doc.byId[b.id].raw).toBe("Title\nold:: new\nbody");
    setBlockProperty(b.id, "old", null);
    expect(doc.byId[b.id].raw).toBe("Title\nbody");
  });
});
