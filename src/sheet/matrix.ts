export interface MatrixCell {
  blockId: string | null;
  row: number;
  col: number;
  rowSpan: number;
  colSpan: number;
}

export interface SheetMatrix {
  rows: number;
  cols: number;
  rowIds: readonly string[];
  cells: readonly MatrixCell[];
}

export function buildMatrix(rows: readonly { id: string; cellIds: readonly string[] }[]): SheetMatrix {
  const rowCount = rows.length;
  let cols = 1;
  for (const row of rows) cols = Math.max(cols, row.cellIds.length);

  const cells: MatrixCell[] = [];
  for (let row = 0; row < rowCount; row++) {
    const cellIds = rows[row].cellIds;
    for (let col = 0; col < cols; col++) {
      cells.push({
        blockId: cellIds[col] ?? null,
        row,
        col,
        rowSpan: 1,
        colSpan: 1,
      });
    }
  }

  return {
    rows: rowCount,
    cols,
    rowIds: rows.map((row) => row.id),
    cells,
  };
}
