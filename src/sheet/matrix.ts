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

export interface MatrixWindowOptions {
  totalRows: number;
  totalCols: number;
  rowStart: number;
  colStart: number;
  visibleCols: number;
}

/** Build only an already-sliced row window. Unlike buildMatrix, this never
 * copies or scans row metadata outside that window. */
export function buildMatrixWindow(
  rows: readonly { id: string; cellIds: readonly string[] }[],
  options: MatrixWindowOptions,
): SheetMatrix {
  const visibleCols = Math.max(0, Math.min(options.totalCols - options.colStart, options.visibleCols));
  const cells: MatrixCell[] = [];
  for (let localRow = 0; localRow < rows.length; localRow++) {
    const row = options.rowStart + localRow;
    const cellIds = rows[localRow].cellIds;
    for (let col = options.colStart; col < options.colStart + visibleCols; col++) {
      cells.push({ blockId: cellIds[col] ?? null, row, col, rowSpan: 1, colSpan: 1 });
    }
  }
  return {
    rows: options.totalRows,
    cols: options.totalCols,
    rowIds: rows.map((row) => row.id),
    cells,
  };
}

const DIMENSION_SCAN_CHUNK = 1_000;
const DIMENSION_INITIAL_SCAN = 200;

interface DimensionEntry {
  key: string;
  rowIds: readonly string[];
  rowCount: number;
  scanned: number;
  cols: number;
  passCols: number;
  scanTimer: ReturnType<typeof setTimeout> | null;
  listeners: Set<(cols: number) => void>;
  cellCountAt: (rowId: string) => number;
}

const dimensionEntries = new Map<string, DimensionEntry>();

function publishDimensions(entry: DimensionEntry, cols: number): void {
  if (cols === entry.cols) return;
  entry.cols = cols;
  for (const listener of entry.listeners) listener(cols);
}

function scanDimensions(entry: DimensionEntry, count: number): void {
  const end = Math.min(entry.rowIds.length, entry.scanned + count);
  let cols = entry.passCols;
  for (let row = entry.scanned; row < end; row++) cols = Math.max(cols, entry.cellCountAt(entry.rowIds[row]));
  entry.passCols = cols;
  entry.scanned = end;
  if (cols > entry.cols || entry.scanned >= entry.rowIds.length) publishDimensions(entry, Math.max(1, cols));
}

function scheduleDimensionScan(entry: DimensionEntry): void {
  if (entry.scanTimer !== null || entry.scanned >= entry.rowIds.length) return;
  entry.scanTimer = setTimeout(() => {
    entry.scanTimer = null;
    if (dimensionEntries.get(entry.key) !== entry || entry.listeners.size === 0) return;
    scanDimensions(entry, DIMENSION_SCAN_CHUNK);
    scheduleDimensionScan(entry);
  }, 0);
}

function cancelDimensionScan(entry: DimensionEntry): void {
  if (entry.scanTimer !== null) clearTimeout(entry.scanTimer);
  entry.scanTimer = null;
}

/** Share the one incremental width discovery walk across duplicate surfaces.
 * The initial synchronous work is capped; the rest yields between chunks. */
export function observeMatrixDimensions(
  key: string,
  rowIds: readonly string[],
  cellCountAt: (rowId: string) => number,
  listener: (cols: number) => void,
): () => void {
  let entry = dimensionEntries.get(key);
  if (!entry || entry.rowIds !== rowIds || entry.rowCount !== rowIds.length) {
    if (entry) cancelDimensionScan(entry);
    entry = { key, rowIds, rowCount: rowIds.length, scanned: 0, cols: 1, passCols: 1, scanTimer: null, listeners: new Set(), cellCountAt };
    dimensionEntries.set(key, entry);
    scanDimensions(entry, DIMENSION_INITIAL_SCAN);
    scheduleDimensionScan(entry);
  }
  entry.listeners.add(listener);
  listener(entry.cols);
  scheduleDimensionScan(entry);
  return () => {
    entry!.listeners.delete(listener);
    if (entry!.listeners.size === 0 && dimensionEntries.get(key) === entry) {
      cancelDimensionScan(entry!);
      dimensionEntries.delete(key);
    }
  };
}

/** Feed widths learned from the active window back into the shared cache. */
export function noteMatrixWindowColumns(key: string, cols: number): void {
  const entry = dimensionEntries.get(key);
  if (entry && cols > entry.cols) publishDimensions(entry, Math.max(1, cols));
}

export function invalidateMatrixDimensions(key: string): void {
  const entry = dimensionEntries.get(key);
  if (!entry) return;
  cancelDimensionScan(entry);
  entry.scanned = 0;
  entry.passCols = 1;
  scanDimensions(entry, DIMENSION_INITIAL_SCAN);
  scheduleDimensionScan(entry);
}

export function invalidateAllMatrixDimensions(): void {
  for (const key of dimensionEntries.keys()) invalidateMatrixDimensions(key);
}

export function clearMatrixDimensionCache(): void {
  for (const entry of dimensionEntries.values()) cancelDimensionScan(entry);
  dimensionEntries.clear();
}

export function matrixDimensionCacheSizeForTests(): number {
  return dimensionEntries.size;
}

export function pendingMatrixDimensionScansForTests(): number {
  let pending = 0;
  for (const entry of dimensionEntries.values()) if (entry.scanTimer !== null) pending++;
  return pending;
}

export interface MatrixBuildOptions {
  visibleRows?: number;
  visibleRowStart?: number;
  visibleCols?: number;
  visibleColStart?: number;
}

export function buildMatrix(
  rows: readonly { id: string; cellIds: readonly string[] }[],
  options: MatrixBuildOptions = {},
): SheetMatrix {
  const rowCount = rows.length;
  let cols = 1;
  for (const row of rows) cols = Math.max(cols, row.cellIds.length);

  const visibleRowStart = Math.max(0, Math.min(Math.max(0, rowCount - 1), options.visibleRowStart ?? 0));
  const visibleRows = Math.max(0, Math.min(rowCount - visibleRowStart, options.visibleRows ?? rowCount));
  const visibleColStart = Math.max(0, Math.min(cols - 1, options.visibleColStart ?? 0));
  const visibleCols = Math.max(0, Math.min(cols - visibleColStart, options.visibleCols ?? cols));
  const cells: MatrixCell[] = [];
  for (let row = visibleRowStart; row < visibleRowStart + visibleRows; row++) {
    const cellIds = rows[row].cellIds;
    for (let col = visibleColStart; col < visibleColStart + visibleCols; col++) {
      const blockId = cellIds[col] ?? null;
      cells.push({ blockId, row, col, rowSpan: 1, colSpan: 1 });
    }
  }

  return {
    rows: rowCount,
    cols,
    rowIds: rows.map((row) => row.id),
    cells,
  };
}
