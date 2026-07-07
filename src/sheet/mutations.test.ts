import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import {
  blockProperty,
  blockSubtreeMarkdown,
  doc,
  loadSingle,
  pageToDto,
  resetStore,
  setDoc,
  undo,
} from "../store";
import type { BlockDto, PageDto } from "../types";
import {
  deleteColumn,
  deleteRow,
  fillSheetSelection,
  insertColumn,
  insertRow,
  materializeCell,
  moveSheetSelection,
  pasteTextIntoSheetSelection,
} from "./mutations";

let counter = 0;
function blk(raw: string, children: BlockDto[] = []): BlockDto {
  return { id: `m${counter++}`, raw, collapsed: false, children };
}

function loadGrid(raw = "Grid\ntine.view:: grid"): string {
  const grid = blk(raw, [
    blk("", [blk("A"), blk("B"), blk("C")]),
    blk("", [blk("D")]),
    blk("", []),
  ]);
  grid.properties = [["tine.view", "grid"]];
  const widths = /(?:^|\n)tine\.col-widths:: ?([^\n]*)/.exec(raw)?.[1];
  if (widths != null) grid.properties.push(["tine.col-widths", widths]);
  const dto: PageDto = { name: "Sheet", kind: "page", title: "Sheet", pre_block: null, blocks: [grid] };
  loadSingle(dto);
  return grid.id;
}

function rows(gridId: string): string[] {
  return [...doc.byId[gridId].children];
}

function rowCells(rowId: string): string[] {
  return doc.byId[rowId].children.map((id) => doc.byId[id].raw);
}

function gridShape(gridId: string): string[][] {
  return rows(gridId).map((rowId) => rowCells(rowId));
}

function cellId(gridId: string, row: number, col: number): string | null {
  const rowId = rows(gridId)[row];
  return rowId ? (doc.byId[rowId].children[col] ?? null) : null;
}

beforeAll(() => initParser());

beforeEach(() => {
  counter = 0;
  resetStore();
});

describe("sheet structural mutations", () => {
  it("inserts an empty row and one undo fully reverts it", () => {
    const gridId = loadGrid();
    const inserted = insertRow(gridId, 1);

    expect(inserted).toBeTruthy();
    expect(rows(gridId)).toHaveLength(4);
    expect(doc.byId[inserted!].raw).toBe("");
    expect(doc.byId[inserted!].children).toEqual([]);
    expect(blockSubtreeMarkdown(gridId, 0, true)).toContain("\t-");

    undo();
    expect(gridShape(gridId)).toEqual([["A", "B", "C"], ["D"], []]);
  });

  it("deletes a row subtree and one undo restores it", () => {
    const gridId = loadGrid();
    const deletedRow = rows(gridId)[0];
    const deletedCell = doc.byId[deletedRow].children[1];

    deleteRow(gridId, 0);

    expect(doc.byId[deletedRow]).toBeUndefined();
    expect(doc.byId[deletedCell]).toBeUndefined();
    expect(gridShape(gridId)).toEqual([["D"], []]);

    undo();
    expect(gridShape(gridId)).toEqual([["A", "B", "C"], ["D"], []]);
  });

  it("inserts a column across ragged rows and shifts col-width keys in the same undo unit", () => {
    const gridId = loadGrid("Grid\ntine.view:: grid\ntine.col-widths:: 0=120;2=88");

    insertColumn(gridId, 1);

    expect(gridShape(gridId)).toEqual([["A", "", "B", "C"], ["D", ""], []]);
    expect(blockProperty(gridId, "tine.col-widths")).toBe("0=120;3=88");

    undo();
    expect(gridShape(gridId)).toEqual([["A", "B", "C"], ["D"], []]);
    expect(blockProperty(gridId, "tine.col-widths")).toBe("0=120;2=88");
  });

  it("deletes a column across ragged rows and rewrites col-width keys in the same undo unit", () => {
    const gridId = loadGrid("Grid\ntine.view:: grid\ntine.col-widths:: 0=120;1=77;2=88");

    deleteColumn(gridId, 1);

    expect(gridShape(gridId)).toEqual([["A", "C"], ["D"], []]);
    expect(blockProperty(gridId, "tine.col-widths")).toBe("0=120;1=88");

    undo();
    expect(gridShape(gridId)).toEqual([["A", "B", "C"], ["D"], []]);
    expect(blockProperty(gridId, "tine.col-widths")).toBe("0=120;1=77;2=88");
  });

  it("materializes a hole by appending exactly the missing cells", () => {
    const gridId = loadGrid();
    const rowId = rows(gridId)[1];

    const cellId = materializeCell(gridId, 1, 3);

    expect(cellId).toBe(doc.byId[rowId].children[3]);
    expect(rowCells(rowId)).toEqual(["D", "", "", ""]);
    expect(blockSubtreeMarkdown(gridId, 0, true)).toContain("\t\t- D");

    undo();
    expect(rowCells(rowId)).toEqual(["D"]);
  });

  it("no-ops cleanly on invalid coordinates", () => {
    const gridId = loadGrid();
    const before = pageToDto("Sheet");

    expect(insertRow(gridId, -1)).toBeNull();
    deleteRow(gridId, 99);
    insertColumn(gridId, 99);
    deleteColumn(gridId, -1);
    expect(materializeCell(gridId, 99, 0)).toBeNull();

    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("moves a cell down into a trailing hole and one undo removes the materialized target", () => {
    const gridId = loadGrid();
    const before = pageToDto("Sheet");

    const next = moveSheetSelection({ kind: "cell", gridId, row: 0, col: 1 }, "down");

    expect(next).toEqual({ kind: "cell", gridId, row: 1, col: 1 });
    expect(gridShape(gridId)).toEqual([["A", "", "C"], ["D", "B"], []]);

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("fills down from the top row raw, strips id::, and one undo removes target holes", () => {
    const gridId = loadGrid();
    const source = cellId(gridId, 0, 1)!;
    setDoc("byId", source, "raw", "B\nid:: hidden");
    const before = pageToDto("Sheet");

    fillSheetSelection(
      { kind: "range", gridId, anchor: { row: 0, col: 1 }, focus: { row: 1, col: 1 } },
      "down"
    );

    const target = cellId(gridId, 1, 1)!;
    expect(doc.byId[target].raw).toBe("B");
    expect(rowCells(rows(gridId)[1])).toEqual(["D", "B"]);

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("pastes TSV from the anchor, grows rows/trailing cells, round-trips, and undoes fully", () => {
    const gridId = loadGrid();
    const before = pageToDto("Sheet");

    const next = pasteTextIntoSheetSelection({ kind: "cell", gridId, row: 2, col: 0 }, "X\tY\nZ\tW");

    expect(next).toEqual({
      kind: "range",
      gridId,
      anchor: { row: 2, col: 0 },
      focus: { row: 3, col: 1 },
    });
    expect(gridShape(gridId)).toEqual([["A", "B", "C"], ["D"], ["X", "Y"], ["Z", "W"]]);
    expect(pageToDto("Sheet")?.blocks[0].children[3].children.map((c) => c.raw)).toEqual(["Z", "W"]);

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });

  it("pastes indented text as nested blocks under the anchor cell", () => {
    const gridId = loadGrid();
    const before = pageToDto("Sheet");

    const next = pasteTextIntoSheetSelection({ kind: "cell", gridId, row: 0, col: 0 }, "parent\n  child");

    expect(next).toEqual({ kind: "cell", gridId, row: 0, col: 0 });
    const anchor = cellId(gridId, 0, 0)!;
    const child = doc.byId[anchor].children[0];
    expect(doc.byId[child].raw).toBe("parent");
    expect(doc.byId[doc.byId[child].children[0]].raw).toBe("child");
    expect(blockSubtreeMarkdown(anchor, 0, true)).toContain("\t- child");

    undo();
    expect(pageToDto("Sheet")).toEqual(before);
  });
});
