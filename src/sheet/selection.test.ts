import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { startEditing, endEdit } from "../editorController";
import { resetStore, selectBlock, setDoc, type FeedPage, type Node as StoreNode } from "../store";
import { initParser } from "../render/parse";
import {
  cellSel,
  handleCellSelectionKey,
  installCellSelectionHooks,
  lastCellFor,
  resetCellSelectionForTests,
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
