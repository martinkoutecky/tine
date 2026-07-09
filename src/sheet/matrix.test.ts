import { describe, expect, it } from "vitest";
import { buildMatrix } from "./matrix";

describe("buildMatrix", () => {
  it("fills trailing holes for ragged rows", () => {
    const matrix = buildMatrix([
      { id: "r1", cellIds: ["a", "b", "c"] },
      { id: "r2", cellIds: ["d"] },
    ]);

    expect(matrix.rows).toBe(2);
    expect(matrix.cols).toBe(3);
    expect(matrix.rowIds).toEqual(["r1", "r2"]);
    expect(matrix.cells.map((c) => c.blockId)).toEqual(["a", "b", "c", "d", null, null]);
    expect(matrix.cells[4]).toMatchObject({ row: 1, col: 1, rowSpan: 1, colSpan: 1 });
  });

  it("keeps empty grids at one column with no cells", () => {
    const matrix = buildMatrix([]);

    expect(matrix.rows).toBe(0);
    expect(matrix.cols).toBe(1);
    expect(matrix.rowIds).toEqual([]);
    expect(matrix.cells).toEqual([]);
  });

  it("renders an empty row as one hole", () => {
    const matrix = buildMatrix([{ id: "r1", cellIds: [] }]);

    expect(matrix.rows).toBe(1);
    expect(matrix.cols).toBe(1);
    expect(matrix.cells).toEqual([{ blockId: null, row: 0, col: 0, rowSpan: 1, colSpan: 1 }]);
  });

  it("handles a single cell", () => {
    const matrix = buildMatrix([{ id: "r1", cellIds: ["c1"] }]);

    expect(matrix.rows).toBe(1);
    expect(matrix.cols).toBe(1);
    expect(matrix.cells).toEqual([{ blockId: "c1", row: 0, col: 0, rowSpan: 1, colSpan: 1 }]);
  });
});
