import { afterEach, describe, expect, it, vi } from "vitest";
import {
  buildMatrix,
  buildMatrixWindow,
  clearMatrixDimensionCache,
  invalidateMatrixDimensions,
  matrixDimensionCacheSizeForTests,
  observeMatrixDimensions,
  pendingMatrixDimensionScansForTests,
} from "./matrix";

afterEach(() => {
  clearMatrixDimensionCache();
  vi.useRealTimers();
});

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

  it("materializes only a leading window while retaining full ragged bounds", () => {
    const rows = Array.from({ length: 500 }, (_, row) => ({
      id: `r${row}`,
      cellIds: row === 499 ? ["last-a", "last-b", "last-c"] : [`c${row}`],
    }));
    const matrix = buildMatrix(rows, { visibleRows: 200, visibleCols: 2 });
    expect(matrix.rows).toBe(500);
    expect(matrix.cols).toBe(3);
    expect(matrix.cells).toHaveLength(400);
    expect(matrix.cells.at(-1)).toMatchObject({ row: 199, col: 1 });
  });

  it("bounds allocation for a very wide grid", () => {
    const matrix = buildMatrix([
      { id: "r1", cellIds: Array.from({ length: 20_000 }, (_, col) => `a${col}`) },
      { id: "r2", cellIds: ["b"] },
      { id: "r3", cellIds: ["c"] },
    ], { visibleRows: 3, visibleCols: 200 });
    expect(matrix.rows).toBe(3);
    expect(matrix.cols).toBe(20_000);
    expect(matrix.cells).toHaveLength(600);
    expect(matrix.cells.at(-1)).toMatchObject({ row: 2, col: 199 });
  });

  it("builds a tall-grid window without copying logical row metadata", () => {
    const matrix = buildMatrixWindow([
      { id: "r50000", cellIds: ["a"] },
      { id: "r50001", cellIds: ["b", "c"] },
    ], {
      totalRows: 100_000,
      totalCols: 3,
      rowStart: 50_000,
      colStart: 0,
      visibleCols: 3,
    });
    expect(matrix.rows).toBe(100_000);
    expect(matrix.rowIds).toEqual(["r50000", "r50001"]);
    expect(matrix.cells).toHaveLength(6);
    expect(matrix.cells[0]).toMatchObject({ row: 50_000, col: 0, blockId: "a" });
    expect(matrix.cells.at(-1)).toMatchObject({ row: 50_001, col: 2, blockId: null });
  });

  it("restarts incomplete discovery after the last observer unmounts", () => {
    vi.useFakeTimers();
    const rows = Array.from({ length: 2_000 }, (_, row) => `r${row}`);
    const width = (id: string) => id === "r1999" ? 7 : 1;
    let cols = 0;
    const stop = observeMatrixDimensions("grid", rows, width, (next) => { cols = next; });
    expect(cols).toBe(1);
    stop();
    vi.runAllTimers();
    expect(matrixDimensionCacheSizeForTests()).toBe(0);

    const stopAgain = observeMatrixDimensions("grid", rows, width, (next) => { cols = next; });
    vi.runAllTimers();
    expect(cols).toBe(7);
    stopAgain();
  });

  it("publishes both shrinking and growing widths after invalidation", () => {
    vi.useFakeTimers();
    const rows = Array.from({ length: 1_200 }, (_, row) => `r${row}`);
    const widths = new Map(rows.map((id) => [id, id === "r1199" ? 5 : 1]));
    let cols = 0;
    const stop = observeMatrixDimensions("grid", rows, (id) => widths.get(id) ?? 0, (next) => { cols = next; });
    vi.runAllTimers();
    expect(cols).toBe(5);

    widths.set("r1199", 1);
    invalidateMatrixDimensions("grid");
    vi.runAllTimers();
    expect(cols).toBe(1);

    widths.set("r1199", 8);
    invalidateMatrixDimensions("grid");
    vi.runAllTimers();
    expect(cols).toBe(8);
    stop();
  });

  it("does not retain tall row arrays after their last observers leave", () => {
    vi.useFakeTimers();
    for (let grid = 0; grid < 50; grid++) {
      const rows = Array.from({ length: 10_000 }, (_, row) => `${grid}:${row}`);
      observeMatrixDimensions(`${grid}`, rows, () => 1, () => {})();
    }
    expect(matrixDimensionCacheSizeForTests()).toBe(0);
    expect(pendingMatrixDimensionScansForTests()).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("cancels discarded-entry and clear-all scan timers", () => {
    vi.useFakeTimers();
    const first = Array.from({ length: 1_000 }, (_, row) => `a${row}`);
    const second = Array.from({ length: 1_000 }, (_, row) => `b${row}`);
    const stopFirst = observeMatrixDimensions("grid", first, () => 1, () => {});
    expect(vi.getTimerCount()).toBe(1);
    observeMatrixDimensions("grid", second, () => 1, () => {});
    expect(vi.getTimerCount()).toBe(1);
    stopFirst();
    clearMatrixDimensionCache();
    expect(matrixDimensionCacheSizeForTests()).toBe(0);
    expect(pendingMatrixDimensionScansForTests()).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });
});
