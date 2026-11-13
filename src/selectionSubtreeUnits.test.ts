import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
const counts = vi.hoisted(() => ({ orders: 0 }));
vi.mock("./document/tree", async original => {
  const tree = await original<typeof import("./document/tree")>();
  return { ...tree,
    pageVisibleOrder: (...args: Parameters<typeof tree.pageVisibleOrder>) => { counts.orders++; return tree.pageVisibleOrder(...args); },
    scopedVisibleOrder: (...args: Parameters<typeof tree.scopedVisibleOrder>) => { counts.orders++; return tree.scopedVisibleOrder(...args); },
    visibleOrder: () => { counts.orders++; return tree.visibleOrder(); },
  };
});
import { initParser } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { clearSelection, deleteSelection, expandBlockSelection, extendSelectionTo, moveSelection, resetStore, selectBlock, selectedIds, selectionMarkdown, singleSelectedId, topSelected, undo, type OutlineScope } from "./document";
import { doc } from "./document/model";
import type { BlockDto } from "./types";
import { ChangeKeyRegistry } from "./changeKeyRegistry";

const block = (id: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({ id, raw: id, children, collapsed });
function load(collapsed = false) {
  loadSingle({ name: "Units", title: "Units", kind: "page", pre_block: null,
    blocks: [block("A"), block("B", [block("B1", [block("B1a")])], collapsed), block("D", [block("D1")]), block("E")] });
}
beforeAll(initParser);
afterEach(resetStore);

describe("complete subtree selection units (GH #941)", () => {
  it.each([false, true])("selecting a parent includes hidden descendants, collapsed=%s", collapsed => {
    load(collapsed);
    selectBlock("B");
    expect(selectedIds()).toEqual(["B", "B1", "B1a"]);
    expect(singleSelectedId()).toBe("B");
    expect(selectionMarkdown()).toBe("- B\n\t- B1\n\t\t- B1a");
  });

  it.each([false, true])("Down extends by a subtree and reverses by a subtree, collapsed=%s", collapsed => {
    load(collapsed);
    selectBlock("B");
    moveSelection(1, true);
    expect(selectedIds()).toEqual(["B", "B1", "B1a", "D", "D1"]);
    moveSelection(1, true);
    expect(selectedIds()).toEqual(["B", "B1", "B1a", "D", "D1", "E"]);
    moveSelection(-1, true);
    expect(selectedIds()).toEqual(["B", "B1", "B1a", "D", "D1"]);
    moveSelection(-1, true);
    expect(selectedIds()).toEqual(["B", "B1", "B1a"]);
  });

  it("Up extends by sibling subtrees and Down shrinks without entering their children", () => {
    load(); selectBlock("E");
    moveSelection(-1, true);
    expect(selectedIds()).toEqual(["D", "D1", "E"]);
    moveSelection(-1, true);
    expect(selectedIds()).toEqual(["B", "B1", "B1a", "D", "D1", "E"]);
    moveSelection(1, true);
    expect(selectedIds()).toEqual(["D", "D1", "E"]);
    moveSelection(1, true);
    expect(selectedIds()).toEqual(["E"]);
  });

  it("reversing an upward selection restores its nested units in order", () => {
    load(); selectBlock("B1a");
    moveSelection(-1, true);
    expect(selectedIds()).toEqual(["B1", "B1a"]);
    moveSelection(-1, true);
    expect(selectedIds()).toEqual(["B", "B1", "B1a"]);
    moveSelection(1, true);
    expect(selectedIds()).toEqual(["B1", "B1a"]);
    moveSelection(1, true);
    expect(selectedIds()).toEqual(["B1a"]);
  });

  it.each([32, 4096])("keypresses reuse view topology on a %i-block page", size => {
    loadSingle({ name: "Cost", title: "Cost", kind: "page", pre_block: null,
      blocks: Array.from({ length: size }, (_, at) => block(`n${at}`)) });
    selectBlock("n1", { roots: [...doc.pages[0].roots] });
    extendSelectionTo(`n${size - 3}`);
    expect(counts.orders).toBeGreaterThan(0);
    counts.orders = 0;
    const started = performance.now();
    for (let at = 0; at < 50; at++) { moveSelection(1, true); moveSelection(-1, true); }
    const elapsed = performance.now() - started;
    expect(selectedIds()).toEqual(Array.from({ length: size - 3 }, (_, at) => `n${at + 1}`));
    expect(counts.orders).toBe(0);
    expect(elapsed).toBeLessThan(500);
  });

  it.each([32, 4096])("extending past a folded parent with %i hidden children notifies independently of them (review round 2)", hidden => {
    loadSingle({ name: "Fold", title: "Fold", kind: "page", pre_block: null, blocks: [block("A"),
      block("B", Array.from({ length: hidden }, (_, at) => block(`h${at}`)), true), block("C")] });
    selectBlock("A");
    const advance = vi.spyOn(ChangeKeyRegistry.prototype, "advance");
    const started = performance.now();
    moveSelection(1, true);
    moveSelection(1, true);
    moveSelection(-1, true);
    const elapsed = performance.now() - started;
    expect(selectedIds()).toHaveLength(hidden + 2);
    expect(advance.mock.calls.length).toBeLessThanOrEqual(16);
    expect(elapsed).toBeLessThan(50);
    advance.mockRestore();
  });

  it.each(["shift-click", "drag"])("%s completes both endpoints across nesting levels", () => {
    load(); selectBlock("B1"); extendSelectionTo("D");
    expect(selectedIds()).toEqual(["B1", "B1a", "D", "D1"]);
    extendSelectionTo("B");
    expect(selectedIds()).toEqual(["B", "B1", "B1a"]);
  });

  it.each<OutlineScope>([
    { roots: ["B"], forceExpandedRoot: "B" },
    { roots: ["B", "D"], collapsed: () => true },
    { roots: ["B", "D"], navOnly: true },
  ])("scope bounds endpoints while selected parents retain source children: %j", scope => {
    load(true); selectBlock("B", scope);
    expect(selectedIds()).toEqual(["B", "B1", "B1a"]);
    expandBlockSelection();
    expect(selectedIds()).not.toContain("E");
    expect(selectedIds()).toContain("B1a");
    extendSelectionTo("E");
    expect(selectedIds()).not.toContain("E");
  });

  it("delete consumes the same subtree as copy and Undo restores it", () => {
    load(true); selectBlock("B"); moveSelection(1, true);
    expect(selectionMarkdown()).toBe("- B\n\t- B1\n\t\t- B1a\n- D\n\t- D1");
    deleteSelection();
    expect(doc.pages[0].roots).toEqual(["A", "E"]);
    expect(selectedIds()).toEqual(["E"]);
    expect(doc.byId.B1a).toBeUndefined();
    undo();
    expect(doc.byId.B1a.parent).toBe("B1");
    clearSelection(); expect(selectedIds()).toEqual([]);
  });

  it("query rows listing a child before its folded parent copy each block once (review F1)", () => {
    loadSingle({ name: "Q", title: "Q", kind: "page", pre_block: null,
      blocks: [block("parent", [block("child")], true)] });
    const scope: OutlineScope = { roots: ["child", "parent"], navOnly: true };
    selectBlock("child", scope); extendSelectionTo("parent", scope);
    expect(topSelected()).toEqual(["parent"]);
    expect(selectionMarkdown()).toBe("- parent\n\t- child");
  });

});
