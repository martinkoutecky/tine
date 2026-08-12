import { splitProps, isBuiltinHidden } from "../editor/properties";
import type { OutlineNode } from "../editor/outline";
import type { Inline, TableBlock } from "../render/ast";
import { visibleBody } from "../render/block";
import { inlineText, parseBody } from "../render/facets";
import { rebulletedSourceByteToRawByte, utf8ByteToUtf16Offset } from "../render/spans";
import {
  applyPageMutationPlan,
  blockIsGridView,
  blockPageReadOnly,
  createPageMutationPlan,
  doc,
  formatForBlock,
  insertOutlineAfter,
  withUndoUnit,
  type PageMutationDraft,
} from "../store";
import { pushToast } from "../ui";
import { sheetConfigFromRaw } from "./config";

type TableCell = Inline[];

interface PipeTableCandidate {
  table: TableBlock;
}

const MAX_PIPE_TABLE_ROWS = 200;
const MAX_PIPE_TABLE_COLS = 30;
let escapedPipeSupport: boolean | null = null;

function tableCandidate(id: string): PipeTableCandidate | null {
  const node = doc.byId[id];
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
  const node = doc.byId[id];
  if (!candidate || !node) return false;
  if (node.children.length > 0) {
    pushToast("Can't convert to grid: children would collide with grid rows.", "error");
    return false;
  }

  const { table } = candidate;
  const rows = table.header ? [table.header, ...table.rows] : table.rows;
  const hasHeader = table.header !== null;
  const page = node.page;
  const raw = node.raw;
  const plan = createPageMutationPlan(page, "sheet:pipe-table-to-grid", (draft) => {
    if (!draft.setRaw(id, rawWithoutTable(raw, table))) return null;
    if (!draft.setProperty(id, "tine.view", "grid")) return null;
    if (!draft.setProperty(id, "tine.header", hasHeader ? "true" : null)) return null;
    for (const row of rows) {
      const rowId = draft.createChild(id, draft.node(id)?.children.length ?? -1);
      if (!rowId) return null;
      for (const cell of row) {
        const cellId = draft.createChild(rowId, draft.node(rowId)?.children.length ?? -1, cellRaw(raw, cell));
        if (!cellId) return null;
      }
    }
    return true;
  });
  return !!plan && applyPageMutationPlan(plan).kind !== "refused";
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
function stripTineSheetProps(draft: PageMutationDraft, id: string): boolean {
  return TINE_SHEET_PROPS.every((key) => draft.setProperty(id, key, null));
}

interface GridTableData {
  rows: string[];
  matrix: string[][];
  header: boolean;
  page: string;
}

function gridTableData(id: string): { ok: true; data: GridTableData } | { ok: false; reason: string } {
  const node = doc.byId[id];
  if (!node || !blockIsGridView(id) || formatForBlock(id) !== "md") {
    return { ok: false, reason: "Convert to pipe table is only available for markdown grids." };
  }
  if (blockPageReadOnly(id)) return { ok: false, reason: "Can't convert a read-only grid to a pipe table." };
  // setBlockProperty can't remove a property that IS line 0 (no title line) —
  // stripping the sheet config would silently fail and leave a half-converted grid.
  if (/^tine\./i.test(node.raw)) {
    return { ok: false, reason: "Can't convert: the grid block needs a title line above its tine.* properties." };
  }

  const rows = [...node.children];
  const colCount = Math.max(0, ...rows.map((rowId) => doc.byId[rowId]?.children.length ?? 0));
  if (rows.length > MAX_PIPE_TABLE_ROWS || colCount > MAX_PIPE_TABLE_COLS) {
    return { ok: false, reason: "Can't convert grids larger than 30 columns by 200 rows to a pipe table." };
  }
  if (rows.length === 0 || colCount === 0) {
    return { ok: false, reason: "Can't convert an empty grid to a pipe table." };
  }

  const matrix: string[][] = [];
  for (const rowId of rows) {
    const row = doc.byId[rowId];
    if (!row || row.page !== node.page) return { ok: false, reason: "Can't convert this grid because a row is missing." };
    const rowSplit = splitProps(row.raw, isBuiltinHidden);
    if (rowSplit.hidden) return { ok: false, reason: "Can't convert: row hidden properties would be lost." };
    if (rowSplit.visible !== "") return { ok: false, reason: "Can't convert: row blocks must be empty." };
    if (row.children.length === 0) {
      return { ok: false, reason: "Can't convert: empty rows cannot be represented as pipe tables." };
    }

    const out: string[] = [];
    for (const cellId of row.children) {
      const cell = doc.byId[cellId];
      if (!cell || cell.page !== node.page) return { ok: false, reason: "Can't convert this grid because a cell is missing." };
      const cellSplit = splitProps(cell.raw, isBuiltinHidden);
      if (cellSplit.hidden) return { ok: false, reason: "Can't convert: cell hidden properties would be lost." };
      if (cell.children.length > 0) return { ok: false, reason: "Can't convert: cells with children cannot be represented in a pipe table." };
      if (cell.raw.includes("\n")) return { ok: false, reason: "Can't convert: cells must be single-line." };
      if (cell.raw.includes("|") && !escapedPipeCellsRoundTrip()) {
        return { ok: false, reason: "Can't convert: this parser does not round-trip \\| inside table cells." };
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
    pushToast(checked.reason, "error");
    return false;
  }
  const { matrix, header, page, rows } = checked.data;
  const table = pipeTable(matrix, header);
  if (table === null) {
    pushToast("Can't convert: this parser does not round-trip \\| inside table cells.", "error");
    return false;
  }

  const plan = createPageMutationPlan(page, "sheet:grid-to-pipe-table", (draft) => {
    if (!stripTineSheetProps(draft, id)) return null;
    const head = trimTrailingBlankLines(draft.node(id)?.raw ?? "");
    if (!draft.setRaw(id, head ? `${head}\n${table}` : table)) return null;
    for (const rowId of rows) if (!draft.deleteSubtree(rowId)) return null;
    return true;
  });
  return !!plan && applyPageMutationPlan(plan).kind !== "refused";
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
  const target = doc.byId[afterId];
  if (!target || blockPageReadOnly(afterId)) return null;
  return withUndoUnit("sheet:drop-delimited-grid", [target.page], () =>
    insertOutlineAfter(afterId, [matrixGridNode(title, matrix)]),
  );
}

export function gridVisibleMatrix(id: string): string[][] {
  return (doc.byId[id]?.children ?? []).map((rowId) =>
    (doc.byId[rowId]?.children ?? []).map((cellId) => visibleBody(doc.byId[cellId]?.raw ?? "").join("\n")),
  );
}
