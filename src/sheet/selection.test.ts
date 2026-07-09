import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { startEditing, endEdit } from "../editorController";
import { doc, isSelected, resetStore, selectBlock, setDoc, type FeedPage, type Node as StoreNode } from "../store";
import { initParser } from "../render/parse";
import {
  cellSel,
  colSeamSel,
  extendCellSelectionTo,
  handleCellSelectionKey,
  installCellSelectionHooks,
  lastCellFor,
  resetCellSelectionForTests,
  rowSeamSel,
  setCellSel,
} from "./selection";

let disposeHooks: (() => void) | null = null;

beforeAll(() => initParser());

beforeEach(() => {
  resetCellSelectionForTests();
});

afterEach(() => {
  disposeHooks?.();
  disposeHooks = null;
  resetCellSelectionForTests();
  resetStore();
  endEdit("blur");
});

function page(roots: string[]): FeedPage {
  return {
    name: "Sheet",
    kind: "page",
    title: "Sheet",
    preBlock: null,
    roots,
    format: "md",
    readOnly: false,
    guide: false,
  };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function loadGrid() {
  setDoc({
    byId: {
      grid: node("grid", "Grid\ntine.view:: grid", null, ["r1", "r2"]),
      r1: node("r1", "", "grid", ["c1", "c2"]),
      c1: node("c1", "A", "r1"),
      c2: node("c2", "B", "r1"),
      r2: node("r2", "", "grid", ["c3", "c4"]),
      c3: node("c3", "C", "r2"),
      c4: node("c4", "D", "r2"),
    },
    pages: [page(["grid"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

function loadNestedGrid() {
  setDoc({
    byId: {
      outer: node("outer", "Outer\ntine.view:: grid", null, ["outer-row"]),
      "outer-row": node("outer-row", "", "outer", ["host"]),
      host: node("host", "Host\ntine.view:: grid", "outer-row", ["inner-row"]),
      "inner-row": node("inner-row", "", "host", ["inner-cell"]),
      "inner-cell": node("inner-cell", "Inner", "inner-row"),
    },
    pages: [page(["outer"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

function press(key: string, init: Partial<KeyboardEvent> = {}) {
  const event = {
    key,
    code: init.code ?? (key === " " ? "Space" : key.startsWith("Arrow") ? key : ""),
    shiftKey: init.shiftKey ?? false,
    ctrlKey: init.ctrlKey ?? false,
    metaKey: init.metaKey ?? false,
    altKey: init.altKey ?? false,
    isComposing: false,
  } as KeyboardEvent;
  expect(handleCellSelectionKey(event)).toBe(true);
}

describe("cell selection state", () => {
  it("sets and clears the active cell while remembering the last cell per grid", () => {
    const clearOutlineSelection = vi.fn();
    const endActiveEdit = vi.fn();
    disposeHooks = installCellSelectionHooks({ clearOutlineSelection, endActiveEdit });

    setCellSel({ gridId: "grid-a", row: 1, col: 2 });

    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid-a", row: 1, col: 2 });
    expect(lastCellFor("grid-a")).toEqual({ row: 1, col: 2 });
    expect(clearOutlineSelection).toHaveBeenCalledTimes(1);
    expect(endActiveEdit).toHaveBeenCalledTimes(1);

    setCellSel({ gridId: "grid-b", row: 0, col: 0 });
    setCellSel(null);

    expect(cellSel()).toBeNull();
    expect(lastCellFor("grid-a")).toEqual({ row: 1, col: 2 });
    expect(lastCellFor("grid-b")).toEqual({ row: 0, col: 0 });
  });

  it("outline selection clears cell selection through the transition hook", () => {
    disposeHooks = installCellSelectionHooks({
      clearOutlineSelection: () => {},
      endActiveEdit: () => {},
    });
    setCellSel({ gridId: "grid-a", row: 0, col: 1 });

    selectBlock("outline-block");

    expect(cellSel()).toBeNull();
  });

  it("non-cell editing clears cell selection, while sheet-cell editing keeps it", () => {
    disposeHooks = installCellSelectionHooks({
      clearOutlineSelection: () => {},
      endActiveEdit: () => {},
    });

    setCellSel({ gridId: "grid-a", row: 0, col: 1 });
    startEditing("cell-block", 0, "sheet:grid-a:0:1");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid-a", row: 0, col: 1 });

    startEditing("outline-block", 0, null);
    expect(cellSel()).toBeNull();
  });

  it("Shift+Arrow promotes a cell to a range and Escape collapses to the anchor", () => {
    loadGrid();
    setCellSel({ gridId: "grid", row: 0, col: 0 });

    press("ArrowRight", { shiftKey: true });
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 0 },
      focus: { row: 0, col: 1 },
    });

    press("ArrowDown", { shiftKey: true });
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 0 },
      focus: { row: 1, col: 1 },
    });

    press("Escape");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });
  });

  it("Shift+Arrow from a col-seam straddles the two cells it divides", () => {
    loadGrid();
    setCellSel(colSeamSel("grid", 1, 0)); // seam between col 0 and col 1, row 0

    press("ArrowRight", { shiftKey: true });
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 0 },
      focus: { row: 0, col: 1 },
    });

    setCellSel(colSeamSel("grid", 1, 0));
    press("ArrowLeft", { shiftKey: true });
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 1 },
      focus: { row: 0, col: 0 },
    });
  });

  it("Shift+Arrow from a row-seam straddles perpendicular, extends along parallel", () => {
    loadGrid();
    setCellSel(rowSeamSel("grid", 1, 0)); // seam between row 0 and row 1, col 0

    press("ArrowDown", { shiftKey: true });
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 0 },
      focus: { row: 1, col: 0 },
    });

    setCellSel(rowSeamSel("grid", 1, 0));
    press("ArrowRight", { shiftKey: true }); // parallel: resolve to the cell on the seam, extend along
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 1, col: 0 },
      focus: { row: 1, col: 1 },
    });
  });

  it("Delete on full-row/full-col selection removes structure; on a partial selection clears contents", () => {
    loadGrid(); // 2×2: A,B / C,D
    // Full row (Shift+Space selects the whole row as a range) → Delete removes it.
    setCellSel({ gridId: "grid", row: 0, col: 0 });
    press(" ", { shiftKey: true });
    press("Backspace");
    expect(doc.byId.grid.children).toHaveLength(1);
    expect(doc.byId[doc.byId.grid.children[0]].children.map((id) => doc.byId[id].raw)).toEqual(["C", "D"]);

    loadGrid();
    // Full column (Ctrl+Space) → Delete removes it.
    setCellSel({ gridId: "grid", row: 0, col: 0 });
    press(" ", { ctrlKey: true });
    press("Delete");
    expect(doc.byId[doc.byId.grid.children[0]].children.map((id) => doc.byId[id].raw)).toEqual(["B"]);
    expect(doc.byId[doc.byId.grid.children[1]].children.map((id) => doc.byId[id].raw)).toEqual(["D"]);

    loadGrid();
    // A lone cell clears contents, never removes structure.
    setCellSel({ gridId: "grid", row: 0, col: 0 });
    press("Backspace");
    expect(doc.byId.grid.children).toHaveLength(2);
    expect(doc.byId.c1.raw).toBe("");
    expect(doc.byId.c2.raw).toBe("B");
  });

  it("mouse range helpers use the same range anchor/focus model as Shift+Arrow", () => {
    loadGrid();
    setCellSel({ gridId: "grid", row: 0, col: 1 });

    expect(extendCellSelectionTo("grid", { row: 1, col: 0 })).toBe(true);
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 1 },
      focus: { row: 1, col: 0 },
    });

    expect(extendCellSelectionTo("grid", { row: 1, col: 1 })).toBe(true);
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 1 },
      focus: { row: 1, col: 1 },
    });
  });

  it("seam selections carry the cell anchor used for movement and insertion", () => {
    loadGrid();
    setCellSel({ gridId: "grid", row: 1, col: 0 });

    press("ArrowRight");
    expect(cellSel()).toEqual(colSeamSel("grid", 1, 1));

    press("ArrowUp");
    expect(cellSel()).toEqual(colSeamSel("grid", 1, 0));

    press("ArrowRight");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 1 });

    press("ArrowDown");
    expect(cellSel()).toEqual(rowSeamSel("grid", 1, 1));
  });

  it("walks the grid seam ladder with visible boundary seams", () => {
    loadGrid();
    setCellSel({ gridId: "grid", row: 1, col: 1 });

    press("ArrowLeft");
    expect(cellSel()).toEqual(colSeamSel("grid", 1, 1));

    press("ArrowLeft");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 1, col: 0 });

    press("ArrowLeft");
    expect(cellSel()).toEqual(colSeamSel("grid", 0, 1));

    press("ArrowLeft");
    expect(cellSel()).toEqual(colSeamSel("grid", 0, 1));

    setCellSel({ gridId: "grid", row: 0, col: 1 });
    press("ArrowRight");
    expect(cellSel()).toEqual(colSeamSel("grid", 2, 0));
    press("ArrowRight");
    expect(cellSel()).toEqual(colSeamSel("grid", 2, 0));

    setCellSel({ gridId: "grid", row: 0, col: 0 });
    press("ArrowUp");
    expect(cellSel()).toEqual(rowSeamSel("grid", 0, 0));
    press("ArrowUp");
    expect(cellSel()).toEqual(rowSeamSel("grid", 0, 0));

    setCellSel({ gridId: "grid", row: 1, col: 0 });
    press("ArrowDown");
    expect(cellSel()).toEqual(rowSeamSel("grid", 2, 0));
    press("ArrowDown");
    expect(cellSel()).toEqual(rowSeamSel("grid", 2, 0));
  });

  it("Escape from a nested grid cell walks up to the containing outer cell, then to outline selection", () => {
    loadNestedGrid();
    setCellSel({ gridId: "host", row: 0, col: 0 });

    press("Escape");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "outer", row: 0, col: 0 });

    press("Escape");
    expect(cellSel()).toBeNull();
    expect(isSelected("outer")).toBe(true);
  });

  it("Escape from nested grid seams walks up to the containing outer cell", () => {
    loadNestedGrid();

    setCellSel(colSeamSel("host", 0, 0));
    press("Escape");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "outer", row: 0, col: 0 });

    setCellSel(rowSeamSel("host", 0, 0));
    press("Escape");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "outer", row: 0, col: 0 });
  });

  it("selects full rows, columns, and the whole grid from cell mode", () => {
    loadGrid();
    setCellSel({ gridId: "grid", row: 1, col: 0 });

    press(" ", { code: "Space", shiftKey: true });
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 1, col: 0 },
      focus: { row: 1, col: 1 },
    });

    setCellSel({ gridId: "grid", row: 0, col: 1 });
    press(" ", { code: "Space", ctrlKey: true });
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 1 },
      focus: { row: 1, col: 1 },
    });

    press("a", { ctrlKey: true });
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 0 },
      focus: { row: 1, col: 1 },
    });
  });
});
