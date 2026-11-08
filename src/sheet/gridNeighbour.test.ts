import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { resetStore } from "../document";
import { setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { initParser } from "../render/parse";
import type { SheetCellCtx } from "./context";
import { containingGridCell, gridEntry, gridNeighbour, type GridMove } from "./gridNeighbour";

// GH #874: grids edit like an outline. The fixture (surface "pane-1"):
//
//   before
//   grid (tine.view:: grid)
//     r1:  a            | b
//                       |   b1
//                       |     b1a
//                       |   b2
//     r2:  c            | d
//            cg (grid)  |   t (table)
//              s1 | s2  |     t1
//              s3       |
//            cz         |
//     r3:  e            (no second column: a hole)
//            e1
//   after
const S = "pane-1";

beforeAll(() => initParser());
beforeEach(load);
afterEach(() => resetStore());

function page(roots: string[]): FeedPage {
  return { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots, format: "md", readOnly: false, guide: false };
}

function n(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function load() {
  setDoc({
    byId: {
      before: n("before", "Before", null),
      grid: n("grid", "Grid\ntine.view:: grid", null, ["r1", "r2", "r3"]),
      r1: n("r1", "", "grid", ["a", "b"]),
      a: n("a", "A", "r1"),
      b: n("b", "B", "r1", ["b1", "b2"]),
      b1: n("b1", "B1", "b", ["b1a"]),
      b1a: n("b1a", "B1a", "b1"),
      b2: n("b2", "B2", "b"),
      r2: n("r2", "", "grid", ["c", "d"]),
      c: n("c", "C", "r2", ["cg", "cz"]),
      cg: n("cg", "Inner\ntine.view:: grid", "c", ["sr1", "sr2"]),
      sr1: n("sr1", "", "cg", ["s1", "s2"]),
      s1: n("s1", "S1", "sr1"),
      s2: n("s2", "S2", "sr1"),
      sr2: n("sr2", "", "cg", ["s3"]),
      s3: n("s3", "S3", "sr2"),
      cz: n("cz", "CZ", "c"),
      d: n("d", "D", "r2", ["t"]),
      t: n("t", "T\ntine.view:: table", "d", ["t1"]),
      t1: n("t1", "T1", "t"),
      r3: n("r3", "", "grid", ["e"]),
      e: n("e", "E", "r3", ["e1"]),
      e1: n("e1", "E1", "e"),
      after: n("after", "After", null),
    },
    pages: [page(["before", "grid", "after"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

const outer = (row: number, col: number): SheetCellCtx => ({ gridId: "grid", row, col, surfaceId: S });
const inner = (row: number, col: number): SheetCellCtx => ({ gridId: "cg", row, col, surfaceId: S });

/** The cell each block of the fixture is edited in. */
const cellOf: Record<string, SheetCellCtx> = {
  a: outer(0, 0), b: outer(0, 1), b1: outer(0, 1), b1a: outer(0, 1), b2: outer(0, 1),
  c: outer(1, 0), cg: outer(1, 0), cz: outer(1, 0), d: outer(1, 1), t: outer(1, 1),
  e: outer(2, 0), e1: outer(2, 0),
  s1: inner(0, 0), s2: inner(0, 1), s3: inner(1, 0),
};

function walk(start: string, move: GridMove): string[] {
  const out: string[] = [];
  let at: { blockId: string; cell: SheetCellCtx | null } = { blockId: start, cell: cellOf[start] };
  while (at.cell) {
    const next = gridNeighbour(at.blockId, at.cell, move);
    if (!next) break;
    out.push(next.blockId);
    if (next.cell) expect(next.cell).toEqual(cellOf[next.blockId]);
    at = next;
  }
  return out;
}

describe("gridNeighbour (GH #874 reading order)", () => {
  it.each(["right", "down"] as const)("body %s skips whole threads in a cell outline", move => {
    setDoc("byId", "comment", n("comment", "Remark\ntine.quote:: B", "b", ["reply"]));
    setDoc("byId", "reply", n("reply", "Reply", "comment"));
    setDoc("byId", "b", "children", ["comment", "b1", "b2"]);
    expect(gridNeighbour("b", outer(0, 1), move)?.blockId).toBe("b1");
    expect(gridNeighbour("comment", outer(0, 1), move)?.blockId).toBe("reply");
    setDoc("byId", "b", "children", ["b1", "comment", "b2"]);
    expect(gridNeighbour("b1a", outer(0, 1), move)?.blockId).toBe("b2");
  });
  it.each(["left", "up"] as const)("body %s skips a preceding thread and a body's trailing thread", move => {
    setDoc("byId", "comment", n("comment", "Remark\ntine.quote:: B", "b", ["reply"]));
    setDoc("byId", "reply", n("reply", "Reply", "comment"));
    setDoc("byId", "b", "children", ["b1", "comment", "b2"]);
    expect(gridNeighbour("b2", outer(0, 1), move)?.blockId).toBe("b1a");
    setDoc("byId", "b", "children", ["b1", "b2", "comment"]);
    expect(gridNeighbour(move === "left" ? "c" : "d", outer(1, move === "left" ? 0 : 1), move)?.blockId).toBe("b2");
  });
  it("entering a grid from below skips a trailing annotation", () => {
    setDoc("byId", "comment", n("comment", "Remark\ntine.quote:: E", "e", ["reply"]));
    setDoc("byId", "reply", n("reply", "Reply", "comment"));
    setDoc("byId", "e", "children", ["e1", "comment"]);
    expect(gridEntry("grid", "below", 1, S)?.blockId).toBe("e1");
  });
  it("retains child navigation across cells when the grid itself belongs to a thread", () => {
    setDoc("byId", "owner", n("owner", "Owner", null, ["thread"]));
    setDoc("byId", "thread", n("thread", "Remark\ntine.quote:: Owner", "owner", ["grid"]));
    setDoc("byId", "grid", "parent", "thread");
    setDoc("pages", 0, "roots", ["owner", "after"]);
    expect(gridNeighbour("c", outer(1, 0), "left")?.blockId).toBe("b2");
    expect(gridNeighbour("d", outer(1, 1), "up")?.blockId).toBe("b2");
    expect(gridEntry("grid", "below", 0, S)?.blockId).toBe("e1");
  });
  it("walks only visible descendants of a collapsed nested block", () => {
    setDoc("byId", "b1", "collapsed", true);
    expect(gridNeighbour("b1", outer(0, 1), "down")?.blockId).toBe("b2");
    expect(gridNeighbour("b2", outer(0, 1), "up")?.blockId).toBe("b1");
    expect(gridNeighbour("b1", outer(0, 1), "right")?.blockId).toBe("b2");
  });
  it("Right walks rows, cells, cell outlines and sub-grids in reading order, then leaves the grid", () => {
    expect(walk("a", "right")).toEqual(["b", "b1", "b1a", "b2", "c", "cg", "s1", "s2", "s3", "cz", "d", "t", "e", "e1", "after"]);
  });

  it("Left is Right's inverse and ends on the grid block's own text", () => {
    expect(walk("e1", "left")).toEqual(["e", "t", "d", "cz", "s3", "s2", "s1", "cg", "c", "b2", "b1a", "b1", "b", "a", "grid"]);
    expect(gridNeighbour("a", outer(0, 0), "left")).toEqual({ blockId: "grid", cell: null, caret: "end" });
  });

  it("Right at a row's end lands at the start of the next row's first cell", () => {
    expect(gridNeighbour("b2", outer(0, 1), "right")).toEqual({ blockId: "c", cell: outer(1, 0), caret: "start" });
    expect(gridNeighbour("c", outer(1, 0), "left")).toEqual({ blockId: "b2", cell: outer(0, 1), caret: "end" });
  });

  it("Down walks a cell's outline, then the cell below in the same column, keeping the caret column", () => {
    expect(gridNeighbour("a", outer(0, 0), "down", 3)).toEqual({ blockId: "c", cell: outer(1, 0), caret: { col: 3, edge: "first" } });
    expect(walk("b", "down")).toEqual(["b1", "b1a", "b2", "d", "t", "after"]);
  });

  it("Down enters a sub-grid at its first cell and leaves it into the rest of the outer cell", () => {
    expect(walk("c", "down")).toEqual(["cg", "s1", "s3", "cz", "e", "e1", "after"]);
    // From the sub-grid's right column, whose bottom row has a hole there.
    expect(gridNeighbour("s2", inner(0, 1), "down")?.blockId).toBe("cz");
  });

  it("Up goes to the last block of the cell above, entering a sub-grid at its bottom-left cell", () => {
    expect(gridNeighbour("d", outer(1, 1), "up", 2)).toEqual({ blockId: "b2", cell: outer(0, 1), caret: { col: 2, edge: "last" } });
    expect(walk("e", "up")).toEqual(["cz", "s3", "s1", "cg", "c", "a", "grid"]);
    expect(gridNeighbour("a", outer(0, 0), "up")).toEqual({ blockId: "grid", cell: null, caret: { col: 0, edge: "last" } });
  });

  it("skips rows that have no cell in the column (holes)", () => {
    expect(gridNeighbour("t", outer(1, 1), "down")?.blockId).toBe("after");
  });

  it("Tab and Shift+Tab move between cells of the same grid, only from a cell's own text", () => {
    expect(gridNeighbour("b", outer(0, 1), "tab-forward")).toEqual({ blockId: "c", cell: outer(1, 0), caret: "start" });
    expect(gridNeighbour("c", outer(1, 0), "tab-back")).toEqual({ blockId: "b", cell: outer(0, 1), caret: "start" });
    expect(gridNeighbour("s2", inner(0, 1), "tab-forward")).toEqual({ blockId: "s3", cell: inner(1, 0), caret: "start" });
    expect(gridNeighbour("e", outer(2, 0), "tab-forward")).toBeNull();
    expect(gridNeighbour("a", outer(0, 0), "tab-back")).toBeNull();
    expect(gridNeighbour("s3", inner(1, 0), "tab-forward")).toBeNull();
    expect(gridNeighbour("b1", outer(0, 1), "tab-forward")).toBeNull();
  });

  it("enters a grid from the outline: Down at the top-left cell, Up at the bottom-left cell's last block", () => {
    expect(gridEntry("grid", "above", 4, S)).toEqual({ blockId: "a", cell: outer(0, 0), caret: { col: 4, edge: "first" } });
    expect(gridEntry("grid", "below", 1, S)).toEqual({ blockId: "e1", cell: outer(2, 0), caret: { col: 1, edge: "last" } });
    expect(gridEntry("before", "above")).toBeNull();
    setDoc("byId", "grid", "collapsed", true);
    expect(gridEntry("grid", "above", 0, S)).toBeNull();
    expect(gridEntry("grid", "below", 0, S)).toBeNull();
  });

  it("finds the cell that holds a block, and refuses blocks inside a table", () => {
    expect(containingGridCell("b1a", S)).toEqual(outer(0, 1));
    expect(containingGridCell("s2", S)).toEqual(inner(0, 1));
    expect(containingGridCell("cg", S)).toEqual(outer(1, 0));
    expect(containingGridCell("before", S)).toBeNull();
    expect(containingGridCell("t1", S)).toBeUndefined();
  });
});
