import { msg, type Msg } from "../i18n";
import { splitProps, BUILTIN_HIDE } from "../editor/properties";
import type { OutlineNode } from "../editor/outline";
import type { Inline, TableBlock } from "../render/ast";
import { visibleBody } from "../render/block";
import { inlineText, parseBody } from "../render/facets";
import { rebulletedSourceByteToRawByte, utf8ByteToUtf16Offset } from "../render/spans";
import { blockIsGridView, blockPageReadOnly, deleteBlock, formatForBlock, insertEmptyChildBlock, insertOutlineAfter, setBlockProperty, setRaw, withUndoUnit, node as docNode } from "../document";
import { pushToast } from "../toasts";
import { sheetConfigFromRaw } from "./config";

type TableCell = Inline[];

interface PipeTableCandidate {
  table: TableBlock;
}

const MAX_PIPE_TABLE_ROWS = 200;
const MAX_PIPE_TABLE_COLS = 30;
let escapedPipeSupport: boolean | null = null;

function tableCandidate(id: string): PipeTableCandidate | null {
  const node = docNode(id);
  if (!node || formatForBlock(id) !== "md" || blockPageReadOnly(id)) return null;
  const blocks = parseBody(node.raw, "md");
  const tables = blocks.filter((b): b is TableBlock => b.kind === "table");
  if (tables.length !== 1 || blocks[blocks.length - 1] !== tables[0] || !tables[0].span) return null;
  return { table: tables[0] };
}

export function canConvertPipeTableToGrid(id: string): boolean {
  return tableCandidate(id) !== null;
}

function rawUtf16SliceFromSourceBytes(raw: string, sourceStart: number, sourceEnd: number): string {
  const rawStart = rebulletedSourceByteToRawByte(raw, sourceStart);
  const rawEnd = rebulletedSourceByteToRawByte(raw, sourceEnd);
  return raw.slice(utf8ByteToUtf16Offset(raw, rawStart), utf8ByteToUtf16Offset(raw, rawEnd));
}

function trimTrailingBlankLines(raw: string): string {
  return raw.replace(/(?:\n[ \t]*)+$/, "");
}

function rawWithoutTable(raw: string, table: TableBlock): string {
  if (!table.span) return raw;
  const start = rebulletedSourceByteToRawByte(raw, table.span[0]);
  const end = rebulletedSourceByteToRawByte(raw, table.span[1]);
  const next = raw.slice(0, utf8ByteToUtf16Offset(raw, start)) + raw.slice(utf8ByteToUtf16Offset(raw, end));
  return trimTrailingBlankLines(next);
}

function cellRaw(raw: string, cell: TableCell): string {
  if (cell.length === 0) return "";
  if (cell.some((inline) => !inline.span)) return inlineText(cell);
  return rawUtf16SliceFromSourceBytes(raw, cell[0].span![0], cell[cell.length - 1].span![1]);
}

export function convertPipeTableToGrid(id: string): boolean {
  const candidate = tableCandidate(id);
  const node = docNode(id);
  if (!candidate || !node) return false;
  if (node.children.length > 0) {
    pushToast(msg("sheet-can-t-convert-to-grid-children-would-collide-with-grid-rows"), "error");
    return false;
  }

  const { table } = candidate;
  const rows = table.header ? [table.header, ...table.rows] : table.rows;
  const hasHeader = table.header !== null;
  const page = node.page;
  return withUndoUnit("sheet:pipe-table-to-grid", [page], () => {
    const raw = docNode(id)?.raw ?? "";
    setRaw(id, rawWithoutTable(raw, table), { timetracking: false });
    setBlockProperty(id, "tine.view", "grid");
    setBlockProperty(id, "tine.header", hasHeader ? "true" : null);
    for (const row of rows) {
      const rowId = insertEmptyChildBlock(id, docNode(id)?.children.length ?? 0);
      if (!rowId) throw new Error("failed to create grid row");
      for (const cell of row) {
        const cellId = insertEmptyChildBlock(rowId, docNode(rowId)?.children.length ?? 0);
        if (!cellId) throw new Error("failed to create grid cell");
        setRaw(cellId, cellRaw(raw, cell), { timetracking: false });
      }
    }
    return true;
  });
}

export function escapedPipeCellsRoundTrip(): boolean {
  if (escapedPipeSupport !== null) return escapedPipeSupport;
  const table = parseBody("T\n| a\\|b | c |\n| --- | --- |\n| x | y |", "md").find(
    (b): b is TableBlock => b.kind === "table",
  );
  escapedPipeSupport = !!table && table.header?.length === 2 && inlineText(table.header[0]) === "a|b";
  return escapedPipeSupport;
}

function tableCellText(raw: string): string | null {
  if (!raw.includes("|")) return raw;
  if (!escapedPipeCellsRoundTrip()) return null;
  return raw.replace(/\|/g, "\\|");
}

function pipeRow(cells: readonly string[]): string | null {
  const out: string[] = [];
  for (const cell of cells) {
    const text = tableCellText(cell);
    if (text === null) return null;
    out.push(text);
  }
  return `| ${out.join(" | ")} |`;
}

function pipeTable(rows: readonly (readonly string[])[], header: boolean): string | null {
  const lines: string[] = [];
  if (header) {
    const head = pipeRow(rows[0] ?? []);
    const sep = pipeRow((rows[0] ?? []).map(() => "---"));
    if (head === null || sep === null) return null;
    lines.push(head, sep);
    for (const row of rows.slice(1)) {
      const line = pipeRow(row);
      if (line === null) return null;
      lines.push(line);
    }
  } else {
    for (const row of rows) {
      const line = pipeRow(row);
      if (line === null) return null;
      lines.push(line);
    }
  }
  return lines.join("\n");
}

const TINE_SHEET_PROPS = ["tine.view", "tine.header", "tine.col-widths", "tine.col-aggregates", "tine.fields", "tine.group-by"];

/** Drop the sheet config properties, keeping every other line of the host
 *  verbatim (a rebuild-from-facets approach would silently lose body lines). */
function stripTineSheetProps(id: string): void {
  for (const key of TINE_SHEET_PROPS) setBlockProperty(id, key, null);
}

function appendTableToHead(id: string, table: string): void {
  const head = trimTrailingBlankLines(docNode(id)?.raw ?? "");
  setRaw(id, head ? `${head}\n${table}` : table, { timetracking: false });
}

interface GridTableData {
  rows: string[];
  matrix: string[][];
  header: boolean;
  page: string;
}

function gridTableData(id: string): { ok: true; data: GridTableData } | { ok: false; reason: Msg } {
  const node = docNode(id);
  if (!node || !blockIsGridView(id) || formatForBlock(id) !== "md") {
    return { ok: false, reason: msg("sheet-grid-requires-markdown") };
  }
  if (blockPageReadOnly(id)) return { ok: false, reason: msg("sheet-grid-read-only") };
  // setBlockProperty can't remove a property that IS line 0 (no title line) —
  // stripping the sheet config would silently fail and leave a half-converted grid.
  if (/^tine\./i.test(node.raw)) {
    return { ok: false, reason: msg("sheet-grid-title-required") };
  }

  // Admit by the row count before copying or visiting any row (I-22): the work
  // below is then bounded by the 200x30 cap, whatever the grid's size.
  const tooLarge = { ok: false as const, reason: msg("sheet-grid-too-large") };
  if (node.children.length > MAX_PIPE_TABLE_ROWS) return tooLarge;
  const rows = [...node.children];
  let colCount = 0;
  for (const rowId of rows) colCount = Math.max(colCount, docNode(rowId)?.children.length ?? 0);
  if (colCount > MAX_PIPE_TABLE_COLS) return tooLarge;
  if (rows.length === 0 || colCount === 0) {
    return { ok: false, reason: msg("sheet-grid-empty") };
  }

  const matrix: string[][] = [];
  for (const rowId of rows) {
    const row = docNode(rowId);
    if (!row || row.page !== node.page) return { ok: false, reason: msg("sheet-grid-row-missing") };
    const rowSplit = splitProps(row.raw, BUILTIN_HIDE);
    if (rowSplit.hidden) return { ok: false, reason: msg("sheet-grid-row-properties") };
    if (rowSplit.visible !== "") return { ok: false, reason: msg("sheet-grid-row-content") };
    if (row.children.length === 0) {
      return { ok: false, reason: msg("sheet-grid-row-empty") };
    }

    const out: string[] = [];
    for (const cellId of row.children) {
      const cell = docNode(cellId);
      if (!cell || cell.page !== node.page) return { ok: false, reason: msg("sheet-grid-cell-missing") };
      const cellSplit = splitProps(cell.raw, BUILTIN_HIDE);
      if (cellSplit.hidden) return { ok: false, reason: msg("sheet-grid-cell-properties") };
      if (cell.children.length > 0) return { ok: false, reason: msg("sheet-grid-cell-children") };
      if (cell.raw.includes("\n")) return { ok: false, reason: msg("sheet-grid-cell-multiline") };
      if (cell.raw.includes("|") && !escapedPipeCellsRoundTrip()) {
        return { ok: false, reason: msg("sheet-grid-cell-pipe") };
      }
      out.push(cell.raw);
    }
    matrix.push(out);
  }

  return {
    ok: true,
    data: {
      rows,
      matrix,
      header: sheetConfigFromRaw(node.raw, formatForBlock(id)).header,
      page: node.page,
    },
  };
}

export function convertGridToPipeTable(id: string): boolean {
  const checked = gridTableData(id);
  if (!checked.ok) {
    pushToast(msg(checked.reason.id), "error");
    return false;
  }
  const { matrix, header, page, rows } = checked.data;
  const table = pipeTable(matrix, header);
  if (table === null) {
    pushToast(msg("sheet-can-t-convert-this-parser-does-not-round-trip-inside-table-cells"), "error");
    return false;
  }

  return withUndoUnit("sheet:grid-to-pipe-table", [page], () => {
    stripTineSheetProps(id);
    appendTableToHead(id, table);
    // Delete the rows where they stand: a verb sees only blocks reachable from
    // the page's roots (outline kernel read set), never a detached subtree.
    for (const rowId of rows) deleteBlock(rowId);
    if (docNode(id)?.children.length) throw new Error("failed to delete grid rows");
    return true;
  });
}

export function delimitedCellCount(matrix: readonly (readonly string[])[]): number {
  return matrix.reduce((sum, row) => sum + row.length, 0);
}

export function matrixGridNode(title: string, matrix: readonly (readonly string[])[]): OutlineNode {
  return {
    raw: `${title || "Dropped table"}\ntine.view:: grid\ntine.header:: true`,
    children: matrix.map((row) => ({
      raw: "",
      children: row.map((cell) => ({ raw: cell, children: [] })),
    })),
  };
}

export function insertMatrixGridAfter(afterId: string, title: string, matrix: readonly (readonly string[])[]): string | null {
  const target = docNode(afterId);
  if (!target || blockPageReadOnly(afterId)) return null;
  return withUndoUnit("sheet:drop-delimited-grid", [target.page], () =>
    insertOutlineAfter(afterId, [matrixGridNode(title, matrix)]),
  );
}

export function gridVisibleMatrix(id: string): string[][] {
  return (docNode(id)?.children ?? []).map((rowId) =>
    (docNode(rowId)?.children ?? []).map((cellId) => visibleBody(docNode(cellId)?.raw ?? "").join("\n")),
  );
}
