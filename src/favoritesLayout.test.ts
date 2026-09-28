// Family 22 (master 7cefc9a7c, d5bb17858, 7b162cb8d): the arrangement model.
import { describe, expect, it } from "vitest";
import type { BlockDto } from "./types";
import {
  type FavLayout, favoriteNode, labelNode, layoutFromBlocks, layoutMembers, layoutToMarkdown,
  moveNode, reconcileLayout, resolveDrop, uniqueGroupName, visibleRows,
} from "./favoritesLayout";

const block = (raw: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({ id: raw, raw, collapsed, children });
const fav = (name: string, children: FavLayout = []) => ({ ...favoriteNode(name), children });
const label = (name: string, children: FavLayout = []) => ({ ...labelNode(name), children });
const names = (layout: FavLayout) => layoutMembers(layout).map((f) => f.name);

describe("page round trip", () => {
  it("reads favorites, labels and nesting to any depth, verbatim", () => {
    const layout = layoutFromBlocks([
      block("[[Alpha]]"),
      block("Work", [block("[[Beta]]"), block("Active", [block("[[Gamma]]")])]),
      block("[[Delta]]", [block("[[Epsilon]]")]),
      block("see [[Zeta]] later"),
      block("  ", [block("[[Kept]]")]),
    ]);
    expect(names(layout)).toEqual(["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Kept"]);
    expect(layout[3]).toMatchObject({ target: null, raw: "see [[Zeta]] later" });
    expect(layoutToMarkdown(layout)).toBe(
      "- [[Alpha]]\n- Work\n\t- [[Beta]]\n\t- Active\n\t\t- [[Gamma]]\n- [[Delta]]\n\t- [[Epsilon]]\n- see [[Zeta]] later\n- [[Kept]]\n");
    expect(layoutToMarkdown([])).toBe("");
  });

  it("classifies a journal title as a journal favorite", () => {
    expect(layoutMembers([favoriteNode("Aug 25th, 2026")])).toEqual([{ name: "Aug 25th, 2026", kind: "journal" }]);
  });
});

describe("reconciling with config.edn membership", () => {
  const layout = [fav("A"), label("Work", [fav("B", [fav("C")])])];
  it("keeps arrangement, promotes a removed favorite's children, keeps labels, appends new members", () => {
    const next = reconcileLayout(layout, ["A", "C", "D"]);
    expect(layoutToMarkdown(next)).toBe("- [[A]]\n- Work\n\t- [[C]]\n- [[D]]\n");
  });
  it("folds case and NFC/NFD spellings and drops duplicates", () => {
    expect(reconcileLayout([], ["Café", "café"])).toHaveLength(1);
  });
  it("names a new label without colliding at any depth", () => {
    expect(uniqueGroupName(layout, "work")).toBe("work 2");
  });
});

describe("rows, drops and moves", () => {
  const layout = [fav("A"), label("Work", [fav("B")]), fav("C")];
  const rows = visibleRows(layout);
  it("walks pre-order with depth and hides a collapsed row's subtree", () => {
    expect(rows.map((r) => [r.node.raw, r.depth])).toEqual([["[[A]]", 0], ["Work", 0], ["[[B]]", 1], ["[[C]]", 0]]);
    expect(visibleRows([{ ...label("Work", [fav("B")]), collapsed: true }])).toHaveLength(1);
  });
  it("clamps depth to one under the row above and no shallower than the row below", () => {
    expect(resolveDrop(rows, 2, 5)).toEqual({ parent: [1], index: 0, depth: 1 });
    expect(resolveDrop(rows, 3, 0)).toEqual({ parent: [], index: 2, depth: 0 });
    expect(resolveDrop(rows, 0, 3)).toEqual({ parent: [], index: 0, depth: 0 });
  });
  it("moves into a label, carries a subtree, reorders post-removal, refuses self-nesting", () => {
    expect(names(moveNode(layout, [0], [1], 1))).toEqual(["B", "A", "C"]);
    expect(layoutToMarkdown(moveNode(layout, [1], [], 2))).toBe("- [[A]]\n- [[C]]\n- Work\n\t- [[B]]\n");
    expect(names(moveNode(layout, [0], [], 1))).toEqual(["B", "A", "C"]);
    expect(moveNode(layout, [1], [1, 0], 0)).toBe(layout);
  });
  it("expands a collapsed row it drops into", () => {
    const moved = moveNode([fav("A"), { ...label("Work", [fav("B")]), collapsed: true }], [0], [1], 0);
    expect(moved[0].collapsed).toBeUndefined();
    expect(names(moved)).toEqual(["A", "B"]);
  });
});
