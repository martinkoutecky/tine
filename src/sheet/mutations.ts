import { blockIsGridView, blockPageReadOnly, blockProperty, blockSubtreeMarkdown, deleteBlock, formatForBlock, insertEmptyChildBlock, insertOutlineChildren, outlineFits, replaceChildOrders, setRaw, pageByName, setBlockProperty, undo, withUndoUnit, node as docNode } from "../document";
import { clipboardWriteRevision, copyRich } from "../clipboard";
import { captureBinding, stillBound } from "../binding";
import { isSheetCellHidden, joinProps, splitProps } from "../editor/properties";
import { parseOutline, type OutlineNode } from "../editor/outline";
import { visibleBody } from "../render/block";
import { pushToast } from "../toasts";
import { serializeColAggregates, serializeColWidths, sheetConfigFromRaw } from "./config";
import type { AggregateFn } from "./aggregate";
import { looksLikeDelimitedText, parseDelimitedText, serializeTsv } from "./tsv";
import type { FieldId } from "./fields";
import { invalidateMatrixDimensions } from "./matrix";

export interface SheetPoint {
  row: number;
  col: number;
}

export interface SheetRect {
  top: number;
  left: number;
  bottom: number;
  right: number;
}

export type SheetMutationSelection =
  | { kind: "cell"; gridId: string; row: number; col: number }
  | { kind: "range"; gridId: string; anchor: SheetPoint; focus: SheetPoint };

export type SheetMoveDirection = "up" | "down" | "left" | "right";

type SheetStructuralCopy = { fingerprint: string; outlineMd: string };

let lastSheetCopy: SheetStructuralCopy | null = null;

const COMPACT_GRID_CONFIG_KEYS = new Set([
  "tine.view",
  "tine.header",
  "tine.col-widths",
  "tine.col-aggregates",
]);

function gridRows(gridId: string): readonly string[] | null {
  if (!blockIsGridView(gridId)) return null;
  return docNode(gridId)?.children ?? null;
}

function gridPage(gridId: string): string | null {
  // null for read-only pages too (the org round-trip gate): every structural
  // grid mutation resolves its page through here, so this is the single choke
  // that keeps sheet writes off pages the block editor already refuses to edit.
  const page = docNode(gridId)?.page ?? null;
  if (page && pageByName(page)?.readOnly) return null;
  return page;
}

function colCount(rows: readonly string[]): number {
  if (rows.length === 0) return 0;
  let cols = 1;
  for (const rowId of rows) cols = Math.max(cols, docNode(rowId)?.children.length ?? 0);
  return cols;
}

export function normalizeSheetRect(a: SheetPoint, b: SheetPoint): SheetRect {
  return {
    top: Math.min(a.row, b.row),
    left: Math.min(a.col, b.col),
    bottom: Math.max(a.row, b.row),
    right: Math.max(a.col, b.col),
  };
}

export function rectForSheetSelection(sel: SheetMutationSelection): SheetRect {
  return sel.kind === "cell"
    ? { top: sel.row, left: sel.col, bottom: sel.row, right: sel.col }
    : normalizeSheetRect(sel.anchor, sel.focus);
}

function offsetPoint(p: SheetPoint, dir: SheetMoveDirection): SheetPoint {
  if (dir === "up") return { row: p.row - 1, col: p.col };
  if (dir === "down") return { row: p.row + 1, col: p.col };
  if (dir === "left") return { row: p.row, col: p.col - 1 };
  return { row: p.row, col: p.col + 1 };
}

function offsetRect(rect: SheetRect, dir: SheetMoveDirection): SheetRect {
  if (dir === "up") return { ...rect, top: rect.top - 1, bottom: rect.bottom - 1 };
  if (dir === "down") return { ...rect, top: rect.top + 1, bottom: rect.bottom + 1 };
  if (dir === "left") return { ...rect, left: rect.left - 1, right: rect.right - 1 };
  return { ...rect, left: rect.left + 1, right: rect.right + 1 };
}

function rectRows(rect: SheetRect): number {
  return rect.bottom - rect.top + 1;
}

function cellIdAt(gridId: string, row: number, col: number): string | null {
  // Coordinate-based mutations are valid only for positional grids. Field
  // tables and boards register with the shared selection layer too, but their
  // screen rows/columns are sorted/query-derived records and fields — they do
  // NOT correspond to owner.children[row].children[col]. Treating them as a
  // grid can silently clear an unrelated nested child on Delete/Cut.
  const rowId = gridRows(gridId)?.[row];
  return rowId ? (docNode(rowId)?.children[col] ?? null) : null;
}

function cellText(blockId: string | null): string {
  const text = blockId ? visibleBody(docNode(blockId)?.raw ?? "").join(" ") : "";
  // The external clipboard flavor is TSV: tabs/newlines are flattened to spaces
  // so a cell body cannot escape into extra external rows or columns.
  return text.replace(/[\t\r\n]+/g, " ");
}

/** Replace a cell's visible text while KEEPING its hidden built-in properties
 *  (id::/collapsed::) and sheet config props: clearing or overwriting a cell must never orphan a
 *  ((ref)) pointing at it (review finding). Fence-aware via splitProps. */
function writeCellVisible(id: string, visible: string): void {
  const fmt = formatForBlock(id);
  const hidden = splitProps(docNode(id)?.raw ?? "", isSheetCellHidden, fmt).hidden;
  const body = splitProps(visible, isSheetCellHidden, fmt).visible;
  setRaw(id, hidden ? joinProps(body, hidden, fmt) : body, { timetracking: false });
}

function rawForCellTransfer(id: string): string {
  const raw = docNode(id)?.raw ?? "";
  return splitProps(raw, isSheetCellHidden, formatForBlock(id)).visible;
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function tableToHtml(rows: readonly (readonly string[])[]): string {
  return `<table><tbody>${rows
    .map((row) => `<tr>${row.map((cell) => `<td>${escapeHtml(cell)}</td>`).join("")}</tr>`)
    .join("")}</tbody></table>`;
}

function ensureGridRows(gridId: string, lastRow: number): boolean {
  let rows = gridRows(gridId);
  if (!rows || lastRow < 0) return false;
  while (rows.length <= lastRow) {
    if (!insertRow(gridId, rows.length)) return false;
    rows = gridRows(gridId);
    if (!rows) return false;
  }
  return true;
}

function ensureRectCells(gridId: string, rect: SheetRect): boolean {
  for (let row = rect.top; row <= rect.bottom; row++) {
    for (let col = rect.left; col <= rect.right; col++) {
      if (!materializeCell(gridId, row, col)) return false;
    }
  }
  return true;
}

function sheetBounds(gridId: string): { rows: number; cols: number } {
  const rows = gridRows(gridId) ?? [];
  return { rows: rows.length, cols: colCount(rows) };
}

function withSheetUndo<T>(gridId: string, tag: string, fn: () => T): T | null {
  // Defense in depth for every coordinate-based compound mutation. Non-grid
  // sheet surfaces must mutate through their adapter's row-id/field semantics.
  if (!gridRows(gridId)) return null;
  const page = gridPage(gridId);
  if (!page) return null;
  return withUndoUnit(tag, [page], fn);
}

function colWidths(gridId: string): ReadonlyMap<number, number> {
  const node = docNode(gridId);
  return node ? sheetConfigFromRaw(node.raw, formatForBlock(gridId)).colWidths : new Map();
}

function colAggregates(gridId: string): ReadonlyMap<string, AggregateFn> {
  const node = docNode(gridId);
  return node ? sheetConfigFromRaw(node.raw, formatForBlock(gridId)).colAggregates : new Map();
}

function writeColWidths(gridId: string, widths: ReadonlyMap<number, number>): void {
  const serialized = serializeColWidths(widths);
  const current = blockProperty(gridId, "tine.col-widths");
  if (serialized === "") {
    if (current !== null) setBlockProperty(gridId, "tine.col-widths", null);
    return;
  }
  if (current === serialized) return;
  setBlockProperty(gridId, "tine.col-widths", serialized);
}

function writeColAggregates(gridId: string, aggregates: ReadonlyMap<string, AggregateFn>): void {
  const serialized = serializeColAggregates(aggregates);
  const current = blockProperty(gridId, "tine.col-aggregates");
  if (serialized === "") {
    if (current !== null) setBlockProperty(gridId, "tine.col-aggregates", null);
    return;
  }
  if (current === serialized) return;
  setBlockProperty(gridId, "tine.col-aggregates", serialized);
}

function shiftedForInsert<T>(values: ReadonlyMap<number, T>, at: number): Map<number, T> {
  const next = new Map<number, T>();
  for (const [col, value] of values) next.set(col >= at ? col + 1 : col, value);
  return next;
}

function shiftedForDelete<T>(values: ReadonlyMap<number, T>, col: number): Map<number, T> {
  const next = new Map<number, T>();
  for (const [idx, value] of values) {
    if (idx === col) continue;
    next.set(idx > col ? idx - 1 : idx, value);
  }
  return next;
}

function indexAggregates(aggregates: ReadonlyMap<string, AggregateFn>): Map<number, AggregateFn> {
  const out = new Map<number, AggregateFn>();
  for (const [key, fn] of aggregates) {
    if (/^\d+$/.test(key)) out.set(Number(key), fn);
  }
  return out;
}

/** Shift the numeric (positional) aggregate keys while carrying the field-keyed
 *  entries (`prop:qty=sum`, `state=count`) through untouched — a grid column op
 *  must never silently drop a field table's aggregate config (review finding). */
function shiftedAggregates(
  aggregates: ReadonlyMap<string, AggregateFn>,
  shift: (m: Map<number, AggregateFn>) => Map<number, AggregateFn>
): Map<string, AggregateFn> {
  const out = new Map<string, AggregateFn>();
  for (const [key, fn] of aggregates) if (!/^\d+$/.test(key)) out.set(key, fn);
  for (const [idx, fn] of shift(indexAggregates(aggregates))) out.set(`${idx}`, fn);
  return out;
}

export function insertRow(gridId: string, at: number): string | null {
  const rows = gridRows(gridId);
  const page = gridPage(gridId);
  if (!rows || !page || at < 0 || at > rows.length) return null;
  const result = withUndoUnit("sheet:insert-row", [page], () => insertEmptyChildBlock(gridId, at));
  if (result) invalidateMatrixDimensions(gridId);
  return result;
}

export function deleteRow(gridId: string, row: number): void {
  const rows = gridRows(gridId);
  const page = gridPage(gridId);
  if (!rows || !page || row < 0 || row >= rows.length) return;
  withUndoUnit("sheet:delete-row", [page], () => deleteBlock(rows[row]));
  invalidateMatrixDimensions(gridId);
}

export function insertColumn(gridId: string, at: number): void {
  const rows = gridRows(gridId);
  const page = gridPage(gridId);
  if (!rows || !page) return;
  const cols = colCount(rows);
  if (at < 0 || at > cols) return;
  withUndoUnit("sheet:insert-column", [page], () => {
    for (const rowId of rows) {
      const row = docNode(rowId);
      if (row && row.children.length >= at) insertEmptyChildBlock(rowId, at);
    }
    writeColWidths(gridId, shiftedForInsert(colWidths(gridId), at));
    writeColAggregates(gridId, shiftedAggregates(colAggregates(gridId), (m) => shiftedForInsert(m, at)));
  });
  invalidateMatrixDimensions(gridId);
}

export function deleteColumn(gridId: string, col: number): void {
  const rows = gridRows(gridId);
  const page = gridPage(gridId);
  if (!rows || !page) return;
  const cols = colCount(rows);
  if (col < 0 || col >= cols) return;
  withUndoUnit("sheet:delete-column", [page], () => {
    for (const rowId of rows) {
      const cellId = docNode(rowId)?.children[col];
      if (cellId) deleteBlock(cellId);
    }
    writeColWidths(gridId, shiftedForDelete(colWidths(gridId), col));
    writeColAggregates(gridId, shiftedAggregates(colAggregates(gridId), (m) => shiftedForDelete(m, col)));
  });
  invalidateMatrixDimensions(gridId);
}

export function deleteRows(gridId: string, top: number, bottom: number): void {
  const rows = gridRows(gridId);
  const page = gridPage(gridId);
  if (!rows || !page) return;
  const lo = Math.max(0, Math.min(top, bottom));
  const hi = Math.min(rows.length - 1, Math.max(top, bottom));
  if (lo > hi) return;
  // Delete high-to-low so the captured row-id snapshot stays valid, all under
  // one undo unit.
  withUndoUnit("sheet:delete-rows", [page], () => {
    for (let r = hi; r >= lo; r--) if (rows[r]) deleteBlock(rows[r]);
  });
  invalidateMatrixDimensions(gridId);
}

export function deleteColumns(gridId: string, left: number, right: number): void {
  const rows = gridRows(gridId);
  const page = gridPage(gridId);
  if (!rows || !page) return;
  const cols = colCount(rows);
  const lo = Math.max(0, Math.min(left, right));
  const hi = Math.min(cols - 1, Math.max(left, right));
  if (lo > hi) return;
  withUndoUnit("sheet:delete-columns", [page], () => {
    for (let c = hi; c >= lo; c--) {
      for (const rowId of rows) {
        const cellId = docNode(rowId)?.children[c];
        if (cellId) deleteBlock(cellId);
      }
      writeColWidths(gridId, shiftedForDelete(colWidths(gridId), c));
      writeColAggregates(gridId, shiftedAggregates(colAggregates(gridId), (m) => shiftedForDelete(m, c)));
    }
  });
  invalidateMatrixDimensions(gridId);
}

export function materializeCell(gridId: string, row: number, col: number): string | null {
  const rows = gridRows(gridId);
  const page = gridPage(gridId);
  if (!rows || !page || row < 0 || row >= rows.length || col < 0) return null;
  const rowId = rows[row];
  const existing = docNode(rowId)?.children[col];
  if (existing) return existing;
  const result = withUndoUnit("sheet:materialize-cell", [page], () => {
    let made: string | null = null;
    while ((docNode(rowId)?.children.length ?? 0) <= col) {
      made = insertEmptyChildBlock(rowId, docNode(rowId)?.children.length ?? 0);
      if (!made) return null;
    }
    return docNode(rowId)?.children[col] ?? made;
  });
  if (result) invalidateMatrixDimensions(gridId);
  return result;
}

export function setColumnWidth(gridId: string, col: number, px: number | null): void {
  const rows = gridRows(gridId);
  const page = gridPage(gridId);
  if (!rows || !page) return;
  const cols = colCount(rows);
  if (col < 0 || col >= cols) return;
  withUndoUnit("sheet:resize-column", [page], () => {
    const next = new Map(colWidths(gridId));
    if (px === null) next.delete(col);
    else next.set(col, Math.max(40, Math.round(px)));
    writeColWidths(gridId, next);
  });
}

export function setColumnAggregate(ownerId: string, key: string, fn: AggregateFn | null): void {
  const node = docNode(ownerId);
  if (!node || !key.trim()) return;
  if (blockPageReadOnly(ownerId)) return; // review finding: footer bypassed the gridPage gate
  withUndoUnit("sheet:column-aggregate", [node.page], () => {
    const next = new Map(colAggregates(ownerId));
    if (fn) next.set(key, fn);
    else next.delete(key);
    writeColAggregates(ownerId, next);
  });
}

/** `""` is an explicit "No grouping" on a query board: a PRESENT empty
 *  `tine.group-field` is the engine's `Cleared`, which blocks the legacy key. */
export function setBoardGroupBy(gridId: string, field: FieldId | ""): void {
  const page = gridPage(gridId);
  if (!page) return;
  if (field === "") {
    withUndoUnit("sheet:group-by", [page], () => {
      setBlockProperty(gridId, "tine.group-field", "");
      if (blockProperty(gridId, "tine.group-by") !== null) setBlockProperty(gridId, "tine.group-by", null);
    });
    return;
  }
  withUndoUnit("sheet:group-by", [page], () => {
    setBlockProperty(gridId, "tine.group-by", field);
    // A query block's Display grouping (`tine.group-field`) outranks the legacy
    // key in the engine's one resolver, so a board choice must move it too or the
    // dropdown would silently do nothing on a block that carries one.
    if (blockProperty(gridId, "tine.group-field") !== null) setBlockProperty(gridId, "tine.group-field", field);
  });
}

export function sheetSelectionText(sel: SheetMutationSelection): { text: string; html: string } {
  if (sel.kind === "cell") {
    const text = cellText(cellIdAt(sel.gridId, sel.row, sel.col));
    return { text, html: escapeHtml(text) };
  }
  const rect = rectForSheetSelection(sel);
  const rows: string[][] = [];
  for (let row = rect.top; row <= rect.bottom; row++) {
    const out: string[] = [];
    for (let col = rect.left; col <= rect.right; col++) out.push(cellText(cellIdAt(sel.gridId, row, col)));
    rows.push(out);
  }
  return { text: serializeTsv(rows), html: tableToHtml(rows) };
}

function emptyOutlineBlock(level: number): string {
  return `${"\t".repeat(level)}-`;
}

function sheetSelectionOutlineMarkdown(sel: SheetMutationSelection): string {
  const rect = rectForSheetSelection(sel);
  const out: string[] = [];
  for (let row = rect.top; row <= rect.bottom; row++) {
    out.push(emptyOutlineBlock(0));
    for (let col = rect.left; col <= rect.right; col++) {
      const id = cellIdAt(sel.gridId, row, col);
      out.push(id ? blockSubtreeMarkdown(id, 1, true) : emptyOutlineBlock(1));
    }
  }
  return out.join("\n");
}

function sheetCellIds(sel: SheetMutationSelection): (string | null)[] {
  const rect = rectForSheetSelection(sel);
  const ids: (string | null)[] = [];
  for (let row = rect.top; row <= rect.bottom; row++) {
    ids.push(gridRows(sel.gridId)?.[row] ?? null);
    for (let col = rect.left; col <= rect.right; col++) ids.push(cellIdAt(sel.gridId, row, col));
  }
  return ids;
}

/** Copy the normalized rectangle as text/HTML (TSV/table for a range, holes
 * empty). After a successful same-graph write, cache its subtree outline for
 * structural paste. Cells stay unchanged. Preparation may throw synchronously;
 * clipboard failure rejects without updating the cache. Cost grows with area
 * and descendant blocks, plus clipboard latency. */
export function copySheetSelection(sel: SheetMutationSelection): Promise<void> {
  const binding = captureBinding();
  const { text, html } = sheetSelectionText(sel);
  const copy = { fingerprint: text, outlineMd: sheetSelectionOutlineMarkdown(sel) };
  const write = copyRich(text, html);
  const ownership = clipboardWriteRevision();
  return write.then(() => { if (stillBound(binding) && clipboardWriteRevision() === ownership) lastSheetCopy = copy; });
}

export function clearSheetSelection(sel: SheetMutationSelection): boolean {
  const rect = rectForSheetSelection(sel);
  return withSheetUndo(sel.gridId, "sheet:clear", () => {
    for (let row = rect.top; row <= rect.bottom; row++) {
      for (let col = rect.left; col <= rect.right; col++) {
        const id = cellIdAt(sel.gridId, row, col);
        if (id) writeCellVisible(id, "");
      }
    }
    return true;
  }) ?? false;
}

/** Copy, then clear visible cell text in one undo unit only while graph,
 * selected IDs, subtree outline, grid and clipboard write token still match.
 * Cells, children and hidden properties stay; selection coordinates do not move.
 * Changed source or read-only page can silently leave a copy. Write/clear errors
 * toast and resolve; preparation before the try may reject. Cost grows with
 * selected area and descendants, plus clipboard I/O and document mutation. */
export async function cutSheetSelection(sel: SheetMutationSelection): Promise<void> {
  const binding = captureBinding();
  const ids = sheetCellIds(sel);
  const before = sheetSelectionOutlineMarkdown(sel);
  try {
    const copy = copySheetSelection(sel);
    const ownership = clipboardWriteRevision();
    await copy;
    if (stillBound(binding)
      && clipboardWriteRevision() === ownership
      && JSON.stringify(sheetCellIds(sel)) === JSON.stringify(ids)
      && sheetSelectionOutlineMarkdown(sel) === before
      && gridRows(sel.gridId)) clearSheetSelection(sel);
  } catch {
    pushToast("Couldn't cut cells: clipboard write failed.", "error");
  }
}

function compactGridConfigSplit(
  raw: string,
  fmt: "md" | "org"
): { visible: string; hidden: string } {
  return splitProps(raw, (key) => COMPACT_GRID_CONFIG_KEYS.has(key.toLowerCase()), fmt);
}

function isCompactGridCell(id: string): boolean {
  return blockIsGridView(id);
}

export function wrapCompactGridCell(cellId: string): string | null {
  const node = docNode(cellId);
  if (!node || !isCompactGridCell(cellId)) return null;
  const rowIds = [...node.children];
  for (const rowId of rowIds) if (!docNode(rowId)) return null;

  const fmt = formatForBlock(cellId);
  const { visible, hidden } = compactGridConfigSplit(node.raw, fmt);
  const hostId = insertEmptyChildBlock(cellId, 0);
  if (!hostId) return null;
  // joinProps wraps the config in a :PROPERTIES: drawer for org; md passes through.
  const config = hidden || (fmt === "org" ? ":tine.view: grid" : "tine.view:: grid");
  setRaw(hostId, joinProps("", config, fmt), { timetracking: false });
  if (!replaceChildOrders({ [cellId]: [hostId], [hostId]: rowIds })) return null;
  setRaw(cellId, visible, { timetracking: false });
  return hostId;
}

export function appendSheetCellChild(cellId: string): string | null {
  const node = docNode(cellId);
  if (!node || blockPageReadOnly(cellId)) return null;
  const result = withUndoUnit("sheet:add-child-bullet", [node.page], () => {
    if (isCompactGridCell(cellId) && !wrapCompactGridCell(cellId)) return false;
    return insertEmptyChildBlock(cellId, docNode(cellId)?.children.length ?? 0) ?? false;
  });
  return result || null;
}

export function fillSheetSelection(sel: SheetMutationSelection, dir: "down" | "right"): boolean {
  const rect = rectForSheetSelection(sel);
  if (dir === "down" && rect.top === rect.bottom) return true;
  if (dir === "right" && rect.left === rect.right) return true;
  return withSheetUndo(sel.gridId, `sheet:fill-${dir}`, () => {
    if (dir === "down") {
      const sources: string[] = [];
      for (let col = rect.left; col <= rect.right; col++) {
        const id = cellIdAt(sel.gridId, rect.top, col);
        sources.push(id ? rawForCellTransfer(id) : "");
      }
      for (let row = rect.top + 1; row <= rect.bottom; row++) {
        for (let col = rect.left; col <= rect.right; col++) {
          const target = materializeCell(sel.gridId, row, col);
          if (target) writeCellVisible(target, sources[col - rect.left]);
        }
      }
      return true;
    }

    const sources: string[] = [];
    for (let row = rect.top; row <= rect.bottom; row++) {
      const id = cellIdAt(sel.gridId, row, rect.left);
      sources.push(id ? rawForCellTransfer(id) : "");
    }
    for (let row = rect.top; row <= rect.bottom; row++) {
      for (let col = rect.left + 1; col <= rect.right; col++) {
        const target = materializeCell(sel.gridId, row, col);
        // writeCellVisible (not bare setRaw) so the target's own hidden id:: survives
        // and no ((ref)) pointing at it is orphaned — same as the fill-down branch.
        if (target) writeCellVisible(target, sources[row - rect.top]);
      }
    }
    return true;
  }) ?? false;
}

function moveWholeRows(gridId: string, rect: SheetRect, dir: "up" | "down"): SheetRect | null {
  const rows = gridRows(gridId);
  if (!rows) return null;
  if (dir === "up" && rect.top <= 0) return null;
  if (dir === "down" && rect.bottom >= rows.length - 1) return null;
  const count = rectRows(rect);
  const next = [...rows];
  const moving = next.splice(rect.top, count);
  const at = dir === "up" ? rect.top - 1 : rect.top + 1;
  next.splice(at, 0, ...moving);
  const ok = withSheetUndo(gridId, "sheet:move-rows", () => replaceChildOrders({ [gridId]: next })) ?? false;
  return ok ? offsetRect(rect, dir) : null;
}

function rotateRowSegment(children: string[], start: number, end: number, dir: "left" | "right"): void {
  if (dir === "left") {
    const first = children[start];
    for (let i = start; i < end; i++) children[i] = children[i + 1];
    children[end] = first;
    return;
  }
  const last = children[end];
  for (let i = end; i > start; i--) children[i] = children[i - 1];
  children[start] = last;
}

function moveRectContent(gridId: string, rect: SheetRect, dir: SheetMoveDirection): SheetRect | null {
  const bounds = sheetBounds(gridId);
  if (bounds.rows <= 0 || bounds.cols <= 0) return null;
  if (dir === "up" && rect.top <= 0) return null;
  if (dir === "down" && rect.bottom >= bounds.rows - 1) return null;
  if (dir === "left" && rect.left <= 0) return null;
  if (dir === "right" && rect.right >= bounds.cols - 1) return null;

  const materialize = { ...rect };
  if (dir === "up") materialize.top--;
  else if (dir === "down") materialize.bottom++;
  else if (dir === "left") materialize.left--;
  else materialize.right++;

  const ok = withSheetUndo(gridId, "sheet:move-range", () => {
    if (!ensureRectCells(gridId, materialize)) return false;

    if (dir === "left" || dir === "right") {
      const nextByParent: Record<string, string[]> = {};
      for (let row = materialize.top; row <= materialize.bottom; row++) {
        const rowId = docNode(gridId)?.children[row];
        if (!rowId) return false;
        const next = [...docNode(rowId).children];
        rotateRowSegment(next, materialize.left, materialize.right, dir);
        nextByParent[rowId] = next;
      }
      return replaceChildOrders(nextByParent);
    }

    const nextByParent: Record<string, string[]> = {};
    const rowIds = docNode(gridId)?.children ?? [];
    for (let row = materialize.top; row <= materialize.bottom; row++) {
      const rowId = rowIds[row];
      if (!rowId) return false;
      nextByParent[rowId] = [...docNode(rowId).children];
    }
    for (let col = materialize.left; col <= materialize.right; col++) {
      if (dir === "up") {
        const first = nextByParent[rowIds[materialize.top]][col];
        for (let row = materialize.top; row < materialize.bottom; row++) {
          nextByParent[rowIds[row]][col] = nextByParent[rowIds[row + 1]][col];
        }
        nextByParent[rowIds[materialize.bottom]][col] = first;
      } else {
        const last = nextByParent[rowIds[materialize.bottom]][col];
        for (let row = materialize.bottom; row > materialize.top; row--) {
          nextByParent[rowIds[row]][col] = nextByParent[rowIds[row - 1]][col];
        }
        nextByParent[rowIds[materialize.top]][col] = last;
      }
    }
    return replaceChildOrders(nextByParent);
  }) ?? false;

  return ok ? offsetRect(rect, dir) : null;
}

export function moveSheetSelection(sel: SheetMutationSelection, dir: SheetMoveDirection): SheetMutationSelection | null {
  const rect = rectForSheetSelection(sel);
  const bounds = sheetBounds(sel.gridId);
  if (sel.kind === "cell") {
    if (!cellIdAt(sel.gridId, sel.row, sel.col)) return null;
    const target = offsetPoint({ row: sel.row, col: sel.col }, dir);
    if (target.row < 0 || target.col < 0 || target.row >= bounds.rows || target.col >= bounds.cols) return null;
    if ((dir === "left" || dir === "right") && !cellIdAt(sel.gridId, target.row, target.col)) return null;
    const moved = moveRectContent(sel.gridId, rect, dir);
    return moved ? { kind: "cell", gridId: sel.gridId, row: target.row, col: target.col } : null;
  }

  if (
    (dir === "up" || dir === "down") &&
    rect.left === 0 &&
    rect.right === Math.max(0, bounds.cols - 1)
  ) {
    const movedRows = moveWholeRows(sel.gridId, rect, dir);
    if (!movedRows) return null;
    const delta = dir === "up" ? -1 : 1;
    return {
      kind: "range",
      gridId: sel.gridId,
      anchor: { row: sel.anchor.row + delta, col: sel.anchor.col },
      focus: { row: sel.focus.row + delta, col: sel.focus.col },
    };
  }

  const moved = moveRectContent(sel.gridId, rect, dir);
  if (!moved) return null;
  const delta = offsetPoint({ row: 0, col: 0 }, dir);
  return {
    kind: "range",
    gridId: sel.gridId,
    anchor: { row: sel.anchor.row + delta.row, col: sel.anchor.col + delta.col },
    focus: { row: sel.focus.row + delta.row, col: sel.focus.col + delta.col },
  };
}

function looksIndentedOutline(text: string): boolean {
  const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  if (!normalized.includes("\n") || normalized.includes("\t")) return false;
  const indents = normalized
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line) => /^ */.exec(line)?.[0].length ?? 0);
  return indents.length > 1 && new Set(indents).size > 1;
}

// When the clipboard text is exactly what we last copied from a grid, hand back
// an OutlineNode that reconstructs the copied cells as a `tine.view:: grid`
// host — so a paste OUTSIDE the grid surface (in a plain block editor) can
// rebuild an actual subgrid instead of dumping the TSV text. Returns null when
// the clipboard isn't our structural copy (caller falls through to text paste).
export function structuralSheetPasteNode(text: string): OutlineNode | null {
  if (!lastSheetCopy || lastSheetCopy.fingerprint !== text) return null;
  const rows = parseOutline(lastSheetCopy.outlineMd);
  if (!rows.length) return null;
  // A single copied cell pastes as plain text, not a 1×1 "subgrid".
  const cellCount = rows.reduce((sum, row) => sum + row.children.length, 0);
  if (cellCount <= 1) return null;
  return { raw: "tine.view:: grid", children: rows };
}

function cellHasVisibleTextOrChildren(id: string): boolean {
  return cellText(id).trim() !== "" || (docNode(id)?.children.length ?? 0) > 0;
}

function pushPasteOverwriteToast(): void {
  pushToast("Pasted over existing cells.", "info", { action: { label: "Undo", run: () => undo() } });
}

export function splatStructuralSheetSelection(
  sel: SheetMutationSelection,
  text: string
): SheetMutationSelection | null | undefined {
  if (!lastSheetCopy || lastSheetCopy.fingerprint !== text) return undefined;
  const rows = parseOutline(lastSheetCopy.outlineMd);
  if (!rows.length) return null;
  const cellCount = rows.reduce((sum, row) => sum + row.children.length, 0);
  if (cellCount <= 1) return undefined;
  const rect = rectForSheetSelection(sel);
  const anchor = { row: rect.top, col: rect.left };
  const height = rows.length;
  const width = Math.max(...rows.map((row) => row.children.length));
  // Every destination cell is two levels below the grid; its imported child
  // starts three levels below. Check all cells before creating rows or clearing
  // any populated destination.
  if (rows.some((row) => row.children.some((cell) => cell.children.length && !outlineFits(sel.gridId, cell.children, 3)))) {
    pushToast("Pasted outline is too deep", "error");
    return null;
  }
  const result =
    withSheetUndo(sel.gridId, "sheet:paste-splat", () => {
      if (!ensureGridRows(sel.gridId, anchor.row + height - 1)) return false;
      let overwroteNonEmpty = false;
      for (let r = 0; r < height; r++) {
        const row = rows[r];
        for (let c = 0; c < row.children.length; c++) {
          const srcCell = row.children[c];
          const target = materializeCell(sel.gridId, anchor.row + r, anchor.col + c);
          if (!target) return false;
          if (cellHasVisibleTextOrChildren(target)) overwroteNonEmpty = true;
          const existingChildren = [...(docNode(target)?.children ?? [])];
          for (const child of existingChildren) deleteBlock(child);
          writeCellVisible(target, srcCell.raw);
          if (srcCell.children.length && !insertOutlineChildren(target, srcCell.children)) return false;
        }
      }
      return { overwroteNonEmpty };
    }) ?? false;
  if (!result) return null;
  if (result.overwroteNonEmpty) pushPasteOverwriteToast();
  return {
    kind: "range",
    gridId: sel.gridId,
    anchor,
    focus: { row: anchor.row + height - 1, col: anchor.col + width - 1 },
  };
}

export function pasteTextIntoSheetSelection(sel: SheetMutationSelection, text: string): SheetMutationSelection | null {
  const rect = rectForSheetSelection(sel);
  const anchor = { row: rect.top, col: rect.left };
  if (looksLikeDelimitedText(text)) {
    const matrix = parseDelimitedText(text);
    if (!matrix.length) return sel;
    const result = withSheetUndo(sel.gridId, "sheet:paste-matrix", () => {
      if (!ensureGridRows(sel.gridId, anchor.row + matrix.length - 1)) return false;
      let overwroteNonEmpty = false;
      for (let r = 0; r < matrix.length; r++) {
        const row = matrix[r];
        for (let c = 0; c < row.length; c++) {
          const existing = cellIdAt(sel.gridId, anchor.row + r, anchor.col + c);
          if (existing && cellHasVisibleTextOrChildren(existing)) overwroteNonEmpty = true;
          const id = materializeCell(sel.gridId, anchor.row + r, anchor.col + c);
          if (!id) return false;
          writeCellVisible(id, row[c]);
        }
      }
      return { overwroteNonEmpty };
    }) ?? false;
    if (!result) return null;
    if (result.overwroteNonEmpty) pushPasteOverwriteToast();
    const height = matrix.length;
    const width = Math.max(1, ...matrix.map((row) => row.length));
    if (height === 1 && width === 1) return { kind: "cell", gridId: sel.gridId, row: anchor.row, col: anchor.col };
    return {
      kind: "range",
      gridId: sel.gridId,
      anchor,
      focus: { row: anchor.row + height - 1, col: anchor.col + width - 1 },
    };
  }

  if (looksIndentedOutline(text)) {
    const nodes = parseOutline(text);
    if (!nodes.length) return sel;
    const ok = withSheetUndo(sel.gridId, "sheet:paste-outline", () => {
      const id = materializeCell(sel.gridId, anchor.row, anchor.col);
      if (!id) return false;
      return !!insertOutlineChildren(id, nodes);
    }) ?? false;
    return ok ? { kind: "cell", gridId: sel.gridId, row: anchor.row, col: anchor.col } : null;
  }

  const ok = withSheetUndo(sel.gridId, "sheet:paste-text", () => {
    const id = materializeCell(sel.gridId, anchor.row, anchor.col);
    if (!id) return false;
    writeCellVisible(id, text.replace(/\r\n/g, "\n").replace(/\r/g, "\n"));
    return true;
  }) ?? false;
  return ok ? { kind: "cell", gridId: sel.gridId, row: anchor.row, col: anchor.col } : null;
}
