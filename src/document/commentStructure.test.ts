// GH #891 / #110, DESIGN §1: OG reads the same child bullets; Tine's body
// operations deliberately treat a whole comment thread as an annotation.
import { afterEach, describe, expect, it } from "vitest";
import {
  canIndentBlock, deleteBlock, extendSelectionTo, indentBlock, indentSelection, insertOutlineAfter, insertOutlineBefore,
  insertOutlineChildren, mergeWithNext, mergeWithPrev, moveBlock, moveBlocksRelative, moveItem,
  node, nextVisible, prevVisible, outdentBlock, outdentSelection, pageByName, pasteClipboardPayload,
  resetStore, selectBlock, selectedIds, moveSelection, setRaw, splitBlock, undo,
} from ".";
import { loadSingle } from "./workingSet";
import { persistCommentMarginHidden } from "../commentMarginPref";
import type { BlockDto, Format } from "../types";
import { blockRegions } from "../render/parse";
import { pasteOutlineInto } from "../tests/pasteOutline";
import { trailingVisibleEmptyLeaf } from "./tree";

const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({
  id, raw, children, collapsed: false,
  properties: blockRegions(raw, format).properties.filter(p => p.primary).map(p => [p.key, p.value]),
});
const PAGE = "Comment structure";
let format: Format = "md";
const quote = () => format === "md" ? "tine.quote:: Parent" : ":PROPERTIES:\n:tine.quote: Parent\n:END:";
function fixture(children?: BlockDto[]) {
  const thread = block("comment", `Remark\n${quote()}`, [block("reply", "Reply", [block("deep", "Deep reply")])]);
  loadSingle({ name: PAGE, title: PAGE, kind: "page", format, pre_block: null, blocks: [
    block("parent", "Parent", children ? children.map(c => c.id === "comment" ? thread : c) : [thread]),
    block("after", "After"),
  ] });
}
const originalThread = () => ["comment", "reply", "deep"].map(id => ({ ...node(id), children: [...node(id).children] }));
afterEach(() => { resetStore(); persistCommentMarginHidden(false); });

describe.each(["md", "org"] as const)("body structure around %s threads", fmt => {
  format = fmt;
  // Set per test because nested describe collection does not select a format.
  function load(children?: BlockDto[]) { format = fmt; fixture(children); }

  it.each([false, true])("Delete skips the whole thread (hidden=%s) and undo restores it", hidden => {
    load([block("comment", ""), block("child", "Child")]);
    persistCommentMarginHidden(hidden);
    const thread = originalThread();
    expect(mergeWithNext("parent")).toBe(true);
    expect(node("parent").raw).toBe("ParentChild");
    expect(originalThread()).toEqual(thread);
    expect(node("parent").children).toEqual(["comment"]);
    undo();
    expect(node("parent").children).toEqual(["comment", "child"]);
    expect(node("child").raw).toBe("Child");
  });

  it("Backspace on the first ordinary child after a thread merges into the parent", () => {
    load([block("comment", ""), block("child", "Child")]);
    const thread = originalThread();
    expect(mergeWithPrev("child")).toBe(true);
    expect(node("parent").raw).toBe("ParentChild");
    expect(originalThread()).toEqual(thread);
  });

  it("Delete after a commented leaf merges the next body block and carries its comments", () => {
    load();
    expect(mergeWithNext("parent")).toBe(true);
    expect(node("parent").raw).toBe("ParentAfter");
    expect(node("parent").children).toEqual(["comment"]);
    expect(node("comment").parent).toBe("parent");
  });

  it.each(["comment", "reply", "deep"])("never merges into or out of %s", id => {
    load();
    const before = originalThread();
    expect(mergeWithPrev(id)).toBe(false);
    expect(mergeWithNext(id)).toBe(false);
    expect(originalThread()).toEqual(before);
  });

  it("Enter with only comments creates a sibling", () => {
    load();
    splitBlock("parent", 6);
    expect(node("parent").children).toEqual(["comment"]);
    expect(pageByName(PAGE)!.roots).toHaveLength(3);
    expect(node(pageByName(PAGE)!.roots[1]).parent).toBeNull();
  });

  it("the body's trailing insertion affordance cannot reuse an empty reply", () => {
    load();
    setRaw("deep", "");
    const scope = { roots: ["parent"] };
    expect(trailingVisibleEmptyLeaf(scope)).toBeNull();
    setRaw("parent", "");
    expect(trailingVisibleEmptyLeaf(scope)).toBe("parent");
  });

  it("Enter with ordinary children inserts a first ordinary child", () => {
    load([block("comment", ""), block("child", "Child")]);
    splitBlock("parent", 6);
    const children = node("parent").children;
    const inserted = children.find(id => id !== "comment" && id !== "child")!;
    expect(node(inserted).raw).toBe("");
    expect(children.indexOf(inserted)).toBeLessThan(children.indexOf("child"));
    expect(node("comment").parent).toBe("parent");
  });

  it("explicit child creation stays outside its parent's thread", () => {
    load();
    splitBlock("parent", 6, true);
    const added = node("parent").children.find(id => id !== "comment")!;
    expect(node(added).parent).toBe("parent");
    expect(node("comment").children).toEqual(["reply"]);
  });

  it.each(["comment", "reply", "deep"])("Tab and Shift+Tab leave %s in its thread, including selections", id => {
    load();
    const before = originalThread();
    expect(canIndentBlock(id)).toBe(false);
    indentBlock(id, 0); outdentBlock(id, 0);
    selectBlock(id); indentSelection(); outdentSelection();
    expect(originalThread()).toEqual(before);
    expect(pageByName(PAGE)!.roots).toEqual(["parent", "after"]);
  });

  it("Tab on an ordinary child cannot nest it inside the preceding thread", () => {
    load([block("comment", ""), block("child", "Child")]);
    indentBlock("child", 0);
    expect(node("child").parent).toBe("parent");
    selectBlock("child"); indentSelection();
    expect(node("child").parent).toBe("parent");
  });

  it("body arrows skip annotation threads; selecting their parent carries the whole subtree (GH #941)", () => {
    load([block("comment", ""), block("child", "Child")]);
    selectBlock("parent"); moveSelection(1, false);
    expect(selectedIds()).toEqual(["child"]);
    selectBlock("parent"); extendSelectionTo("child");
    expect(selectedIds()).toEqual(["parent", "comment", "reply", "deep", "child"]);
  });

  it("the shared navigation API skips threads from body blocks and retains in-thread steps", () => {
    load([block("comment", ""), block("child", "Child")]);
    expect(nextVisible("parent")).toBe("child");
    expect(prevVisible("child")).toBe("parent");
    expect(nextVisible("parent", { roots: ["parent"] })).toBe("child");
    expect(nextVisible("comment")).toBe("reply");
    expect(prevVisible("reply")).toBe("comment");
  });

  it.each([false, true])("Tab skips an intervening thread to find the previous ordinary sibling (hidden=%s)", hidden => {
    load([block("previous", "Previous"), block("comment", ""), block("child", "Child")]);
    persistCommentMarginHidden(hidden);
    indentBlock("child", 0);
    expect(node("child").parent).toBe("previous");
    expect(node("comment").parent).toBe("parent");
    undo();
    selectBlock("child", { roots: ["parent", "after"] }); indentSelection();
    expect(node("child").parent).toBe("previous");
    expect(node("comment").parent).toBe("parent");
  });

  it("traditional outdent carries ordinary following siblings and leaves the parent's comment there", () => {
    load([block("child", "Child"), block("following", "Following"), block("comment", "")]);
    outdentBlock("child", 0);
    expect(node("child").parent).toBeNull();
    expect(node("following").parent).toBe("child");
    expect(node("comment").parent).toBe("parent");
  });

  it.each(["comment", "reply", "deep"])("drag rejects %s as a source and target", async id => {
    load();
    const before = originalThread();
    expect(await moveBlocksRelative([id], "after", "before")).toBe(false);
    for (const position of ["before", "after", "child"] as const) {
      expect(await moveBlocksRelative(["after"], id, position)).toBe(false);
    }
    await moveBlock("after", id, 0);
    await moveBlock("after", "parent", 0, PAGE, id);
    expect(node("after").parent).toBeNull();
    expect(originalThread()).toEqual(before);
  });

  it("a keyboard move across a parent's edge cannot enter its sibling thread", () => {
    load([block("ordinary", "Ordinary", [block("leaf", "Leaf")]), block("comment", "")]);
    expect(moveItem("leaf", 1)).toBe(false);
    expect(node("leaf").parent).toBe("ordinary");
    expect(node("comment").children).toEqual(["reply"]);
  });

  it("moving or deleting the commented body block still takes its thread", async () => {
    load();
    expect(await moveBlocksRelative(["parent"], "after", "child")).toBe(true);
    expect(node("comment").parent).toBe("parent");
    expect(node("parent").parent).toBe("after");
    deleteBlock("parent");
    expect(node("comment")).toBeUndefined();
    expect(node("deep")).toBeUndefined();
    undo();
    expect(node("comment").parent).toBe("parent");
    expect(node("deep").parent).toBe("reply");
  });

  it.each(["comment", "reply", "deep"])("outline paste never lands in %s", id => {
    load();
    const pasted = [{ raw: "Pasted", children: [] }];
    const before = originalThread();
    expect(insertOutlineAfter(id, pasted)).toBeNull();
    expect(insertOutlineBefore(id, pasted)).toBeNull();
    expect(insertOutlineChildren(id, pasted)).toBeNull();
    expect(pasteOutlineInto(id, pasted)).toBeNull();
    expect(originalThread()).toEqual(before);
  });

  it("body paste beside a commented block stays its sibling; child paste stays outside the thread", () => {
    load();
    const sibling = insertOutlineAfter("parent", [{ raw: "Sibling", children: [] }])!;
    const child = insertOutlineChildren("parent", [{ raw: "Child", children: [] }])!;
    expect(node(sibling).parent).toBeNull();
    expect(node(child).parent).toBe("parent");
    expect(blockRegions(node("comment").raw, fmt).properties.some(p => p.key === "tine.quote")).toBe(true);
  });

  it("private clipboard paste refuses thread targets and still pastes beside body blocks", async () => {
    load();
    const slot = {
      op: "copy" as const, generation: 1, graph: "", text: "Pasted", sourcePages: [],
      blocks: [{ raw: "Pasted", sourceFormat: fmt, children: [] }],
    };
    for (const target of ["comment", "reply", "deep"]) {
      expect(await pasteClipboardPayload(target, slot)).toBeNull();
    }
    const inserted = await pasteClipboardPayload("parent", slot);
    expect(inserted).not.toBeNull();
    expect(node(inserted!).parent).toBeNull();
  });
});
