import {
  blockIsGridView,
  blockPageReadOnly,
  blockProperty,
  blockSubtreeMarkdown,
  doc,
  formatForBlock,
  insertEmptyChildBlock,
  replaceChildOrders,
  setRaw,
  pageByName,
  setBlockProperty,
  createPageMutationPlan,
  applyPageMutationPlan,
  type PageMutationDraft,
  undo,
  withUndoUnit,
} from "../store";
import { copyRich } from "../clipboard";
import { isSheetCellHidden, joinProps, splitProps } from "../editor/properties";
import { parseOutline, type OutlineNode } from "../editor/outline";
import { visibleBody } from "../render/block";
import { pushToast } from "../ui";
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

function gridRows(gridId: string): string[] | null {
  if (!blockIsGridView(gridId)) return null;
  return doc.byId[gridId]?.children ?? null;
}

function gridPage(gridId: string): string | null {
  // null for read-only pages too (the org round-trip gate): every structural
  // grid mutation resolves its page through here, so this is the single choke
  // that keeps sheet writes off pages the block editor already refuses to edit.
  const page = doc.byId[gridId]?.page ?? null;
  if (page && pageByName(page)?.readOnly) return null;
  return page;
}

function colCount(rows: readonly string[]): number {
  if (rows.length === 0) return 0;
  let cols = 1;
  for (const rowId of rows) cols = Math.max(cols, doc.byId[rowId]?.children.length ?? 0);
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

export function focusForSheetSelection(sel: SheetMutationSelection): SheetPoint {
  return sel.kind === "cell" ? { row: sel.row, col: sel.col } : { ...sel.focus };
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
  return rowId ? (doc.byId[rowId]?.children[col] ?? null) : null;
}

function cellText(blockId: string | null): string {
  const text = blockId ? visibleBody(doc.byId[blockId]?.raw ?? "").join(" ") : "";
  // The external clipboard flavor is TSV: tabs/newlines are flattened to spaces
  // so a cell body cannot escape into extra external rows or columns.
  return text.replace(/[\t\r\n]+/g, " ");
}

/** Replace a cell's visible text while KEEPING its hidden built-in properties
 *  (id::/collapsed::) and sheet config props: clearing or overwriting a cell must never orphan a
 *  ((ref)) pointing at it (review finding). Fence-aware via splitProps. */
function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function tableToHtml(rows: readonly (readonly string[])[]): string {
  return `<table><tbody>${rows
    .map((row) => `<tr>${row.map((cell) => `<td>${escapeHtml(cell)}</td>`).join("")}</tr>`)
    .join("")}</tbody></table>`;
}

function sheetBounds(gridId: string): { rows: number; cols: number } {
  const rows = gridRows(gridId) ?? [];
  return { rows: rows.length, cols: colCount(rows) };
}

function plannedSheetMutation<T>(
  gridId: string,
  tag: string,
  build: (draft: PageMutationDraft) => T | null,
  afterApply?: (value: T) => void,
): T | null {
  if (!gridRows(gridId)) return null;
  const page = gridPage(gridId);
  if (!page) return null;
  const plan = createPageMutationPlan(page, tag, build);
  if (!plan) return null;
  applyPageMutationPlan(plan, afterApply);
  return plan.value;
}

function draftRows(draft: PageMutationDraft, gridId: string): string[] | null {
  return draft.node(gridId)?.children ?? null;
}

function draftColCount(draft: PageMutationDraft, gridId: string): number {
  const rows = draftRows(draft, gridId) ?? [];
  let cols = rows.length ? 1 : 0;
  for (const rowId of rows) cols = Math.max(cols, draft.node(rowId)?.children.length ?? 0);
  return cols;
}

function draftCellId(draft: PageMutationDraft, gridId: string, row: number, col: number): string | null {
  const rowId = draftRows(draft, gridId)?.[row];
  return rowId ? (draft.node(rowId)?.children[col] ?? null) : null;
}

function draftWriteVisible(draft: PageMutationDraft, id: string, visible: string): boolean {
  const node = draft.node(id);
  if (!node) return false;
  const hidden = splitProps(node.raw, isSheetCellHidden, draft.page.format).hidden;
  return draft.setRaw(id, hidden ? joinProps(visible, hidden, draft.page.format) : visible);
}

function draftRawWithoutId(draft: PageMutationDraft, id: string): string {
  return splitProps(
    draft.node(id)?.raw ?? "",
    (key) => key.toLowerCase() === "id",
    draft.page.format,
  ).visible;
}

function draftMaterializeCell(
  draft: PageMutationDraft,
  gridId: string,
  row: number,
  col: number,
): string | null {
  const rowId = draftRows(draft, gridId)?.[row];
  if (!rowId || col < 0) return null;
  const existing = draft.node(rowId)?.children[col];
  if (existing) return existing;
  let made: string | null = null;
  while ((draft.node(rowId)?.children.length ?? 0) <= col) {
    made = draft.createChild(rowId, draft.node(rowId)?.children.length ?? 0);
    if (!made) return null;
  }
  return draft.node(rowId)?.children[col] ?? made;
}

function draftEnsureRows(draft: PageMutationDraft, gridId: string, lastRow: number): boolean {
  const rows = draftRows(draft, gridId);
  if (!rows || lastRow < 0) return false;
  while (rows.length <= lastRow) if (!draft.createChild(gridId, rows.length)) return false;
  return true;
}

function draftEnsureRect(draft: PageMutationDraft, gridId: string, rect: SheetRect): boolean {
  for (let row = rect.top; row <= rect.bottom; row++) {
    for (let col = rect.left; col <= rect.right; col++) {
      if (!draftMaterializeCell(draft, gridId, row, col)) return false;
    }
  }
  return true;
}

function colWidths(gridId: string): ReadonlyMap<number, number> {
  const node = doc.byId[gridId];
  return node ? sheetConfigFromRaw(node.raw, formatForBlock(gridId)).colWidths : new Map();
}

function colAggregates(gridId: string): ReadonlyMap<string, AggregateFn> {
  const node = doc.byId[gridId];
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

export function insertRow(gridId: string, at: number, afterApply?: (id: string) => void): string | null {
  const result = plannedSheetMutation(gridId, "sheet:insert-row", (draft) => {
    const rows = draftRows(draft, gridId);
    if (!rows || at < 0 || at > rows.length) return null;
    return draft.createChild(gridId, at);
  }, (id) => {
    invalidateMatrixDimensions(gridId);
    afterApply?.(id);
  });
  return result;
}

export function deleteRow(gridId: string, row: number, afterApply?: () => void): void {
  plannedSheetMutation(gridId, "sheet:delete-row", (draft) => {
    const id = draftRows(draft, gridId)?.[row];
    return id && draft.deleteSubtree(id) ? true : null;
  }, () => {
    invalidateMatrixDimensions(gridId);
    afterApply?.();
  });
}

export function insertColumn(gridId: string, at: number, afterApply?: () => void): void {
  plannedSheetMutation(gridId, "sheet:insert-column", (draft) => {
    const rows = draftRows(draft, gridId);
    if (!rows || at < 0 || at > draftColCount(draft, gridId)) return null;
    for (const rowId of rows) {
      const row = draft.node(rowId);
      if (row && row.children.length >= at && !draft.createChild(rowId, at)) return null;
    }
    const config = sheetConfigFromRaw(draft.node(gridId)?.raw ?? "", draft.page.format);
    const widths = serializeColWidths(shiftedForInsert(config.colWidths, at));
    const aggregates = serializeColAggregates(
      shiftedAggregates(config.colAggregates, (m) => shiftedForInsert(m, at)),
    );
    if (!draft.setProperty(gridId, "tine.col-widths", widths || null)) return null;
    if (!draft.setProperty(gridId, "tine.col-aggregates", aggregates || null)) return null;
    return true;
  }, () => {
    invalidateMatrixDimensions(gridId);
    afterApply?.();
  });
}

export function deleteColumn(gridId: string, col: number, afterApply?: () => void): void {
  plannedSheetMutation(gridId, "sheet:delete-column", (draft) => {
    const rows = draftRows(draft, gridId);
    if (!rows || col < 0 || col >= draftColCount(draft, gridId)) return null;
    for (const rowId of rows) {
      const cellId = draft.node(rowId)?.children[col];
      if (cellId && !draft.deleteSubtree(cellId)) return null;
    }
    const config = sheetConfigFromRaw(draft.node(gridId)?.raw ?? "", draft.page.format);
    const widths = serializeColWidths(shiftedForDelete(config.colWidths, col));
    const aggregates = serializeColAggregates(
      shiftedAggregates(config.colAggregates, (m) => shiftedForDelete(m, col)),
    );
    if (!draft.setProperty(gridId, "tine.col-widths", widths || null)) return null;
    if (!draft.setProperty(gridId, "tine.col-aggregates", aggregates || null)) return null;
    return true;
  }, () => {
    invalidateMatrixDimensions(gridId);
    afterApply?.();
  });
}

export function deleteRows(gridId: string, top: number, bottom: number, afterApply?: () => void): void {
  plannedSheetMutation(gridId, "sheet:delete-rows", (draft) => {
    const rows = draftRows(draft, gridId);
    if (!rows) return null;
    const lo = Math.max(0, Math.min(top, bottom));
    const hi = Math.min(rows.length - 1, Math.max(top, bottom));
    if (lo > hi) return null;
    const selected = rows.slice(lo, hi + 1);
    for (let index = selected.length - 1; index >= 0; index--) {
      if (!draft.deleteSubtree(selected[index])) return null;
    }
    return true;
  }, () => {
    invalidateMatrixDimensions(gridId);
    afterApply?.();
  });
}

export function deleteColumns(gridId: string, left: number, right: number, afterApply?: () => void): void {
  plannedSheetMutation(gridId, "sheet:delete-columns", (draft) => {
    const rows = draftRows(draft, gridId);
    if (!rows) return null;
    const cols = draftColCount(draft, gridId);
    const lo = Math.max(0, Math.min(left, right));
    const hi = Math.min(cols - 1, Math.max(left, right));
    if (lo > hi) return null;
    let widths = new Map(sheetConfigFromRaw(draft.node(gridId)?.raw ?? "", draft.page.format).colWidths);
    let aggregates = new Map(sheetConfigFromRaw(draft.node(gridId)?.raw ?? "", draft.page.format).colAggregates);
    for (let c = hi; c >= lo; c--) {
      for (const rowId of rows) {
        const cellId = draft.node(rowId)?.children[c];
        if (cellId && !draft.deleteSubtree(cellId)) return null;
      }
      widths = shiftedForDelete(widths, c);
      aggregates = shiftedAggregates(aggregates, (m) => shiftedForDelete(m, c));
    }
    if (!draft.setProperty(gridId, "tine.col-widths", serializeColWidths(widths) || null)) return null;
    if (!draft.setProperty(gridId, "tine.col-aggregates", serializeColAggregates(aggregates) || null)) return null;
    return true;
  }, () => {
    invalidateMatrixDimensions(gridId);
    afterApply?.();
  });
}

export function materializeCell(
  gridId: string,
  row: number,
  col: number,
  afterApply?: (cellId: string) => void,
): string | null {
  const existing = cellIdAt(gridId, row, col);
  if (existing) {
    afterApply?.(existing);
    return existing;
  }
  return plannedSheetMutation(gridId, "sheet:materialize-cell", (draft) =>
    draftMaterializeCell(draft, gridId, row, col), (cellId) => {
      invalidateMatrixDimensions(gridId);
      afterApply?.(cellId);
    });
}

/** Atomic seam insertion plus the cell required for its post-commit edit
 * target. This avoids the old row/column publication followed by a second
 * materialization publication. */
export function insertSheetSeam(
  gridId: string,
  kind: "row" | "col",
  at: number,
  anchor: number,
  afterApply?: (target: SheetPoint) => void,
): SheetPoint | null {
  return plannedSheetMutation(gridId, "sheet:seam-insert", (draft) => {
    const rows = draftRows(draft, gridId);
    if (!rows) return null;
    if (kind === "row") {
      if (at < 0 || at > rows.length || !draft.createChild(gridId, at)) return null;
      const col = Math.max(0, anchor);
      if (!draftMaterializeCell(draft, gridId, at, col)) return null;
      return { row: at, col };
    }
    const cols = draftColCount(draft, gridId);
    if (at < 0 || at > cols) return null;
    for (const rowId of rows) {
      const row = draft.node(rowId);
      if (row && row.children.length >= at && !draft.createChild(rowId, at)) return null;
    }
    const config = sheetConfigFromRaw(draft.node(gridId)?.raw ?? "", draft.page.format);
    if (!draft.setProperty(
      gridId,
      "tine.col-widths",
      serializeColWidths(shiftedForInsert(config.colWidths, at)) || null,
    )) return null;
    if (!draft.setProperty(
      gridId,
      "tine.col-aggregates",
      serializeColAggregates(shiftedAggregates(config.colAggregates, (m) => shiftedForInsert(m, at))) || null,
    )) return null;
    const row = Math.max(0, anchor);
    if (!draftMaterializeCell(draft, gridId, row, at)) return null;
    return { row, col: at };
  }, (target) => {
    invalidateMatrixDimensions(gridId);
    afterApply?.(target);
  });
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
  const node = doc.byId[ownerId];
  if (!node || !key.trim()) return;
  if (blockPageReadOnly(ownerId)) return; // review finding: footer bypassed the gridPage gate
  withUndoUnit("sheet:column-aggregate", [node.page], () => {
    const next = new Map(colAggregates(ownerId));
    if (fn) next.set(key, fn);
    else next.delete(key);
    writeColAggregates(ownerId, next);
  });
}

export function setBoardGroupBy(gridId: string, field: FieldId): void {
  const page = gridPage(gridId);
  if (!page) return;
  withUndoUnit("sheet:group-by", [page], () => setBlockProperty(gridId, "tine.group-by", field));
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

export function copySheetSelection(sel: SheetMutationSelection): Promise<void> {
  const { text, html } = sheetSelectionText(sel);
  lastSheetCopy = { fingerprint: text, outlineMd: sheetSelectionOutlineMarkdown(sel) };
  return copyRich(text, html);
}

export function clearSheetSelection(sel: SheetMutationSelection, afterApply?: () => void): boolean {
  const rect = rectForSheetSelection(sel);
  const result = plannedSheetMutation(sel.gridId, "sheet:clear", (draft) => {
    for (let row = rect.top; row <= rect.bottom; row++) {
      for (let col = rect.left; col <= rect.right; col++) {
        const id = draftCellId(draft, sel.gridId, row, col);
        if (id && !draftWriteVisible(draft, id, "")) return null;
      }
    }
    return true;
  }, () => afterApply?.());
  return result ?? false;
}

export function cutSheetSelection(sel: SheetMutationSelection, afterApply?: () => void): void {
  void copySheetSelection(sel);
  clearSheetSelection(sel, afterApply);
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
  const node = doc.byId[cellId];
  if (!node || !isCompactGridCell(cellId)) return null;
  const rowIds = [...node.children];
  for (const rowId of rowIds) if (!doc.byId[rowId]) return null;

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
  const node = doc.byId[cellId];
  if (!node || blockPageReadOnly(cellId)) return null;
  return withUndoUnit("sheet:add-child-bullet", [node.page], () => {
    if (isCompactGridCell(cellId) && !wrapCompactGridCell(cellId)) return null;
    return insertEmptyChildBlock(cellId, doc.byId[cellId]?.children.length ?? 0);
  });
}

export function fillSheetSelection(
  sel: SheetMutationSelection,
  dir: "down" | "right",
  afterApply?: () => void,
): boolean {
  const rect = rectForSheetSelection(sel);
  if (dir === "down" && rect.top === rect.bottom) return true;
  if (dir === "right" && rect.left === rect.right) return true;
  const result = plannedSheetMutation(sel.gridId, `sheet:fill-${dir}`, (draft) => {
    if (dir === "down") {
      const sources: string[] = [];
      for (let col = rect.left; col <= rect.right; col++) {
        const id = draftCellId(draft, sel.gridId, rect.top, col);
        sources.push(id ? draftRawWithoutId(draft, id) : "");
      }
      for (let row = rect.top + 1; row <= rect.bottom; row++) {
        for (let col = rect.left; col <= rect.right; col++) {
          const target = draftMaterializeCell(draft, sel.gridId, row, col);
          if (!target || !draftWriteVisible(draft, target, sources[col - rect.left])) return null;
        }
      }
      return true;
    }

    const sources: string[] = [];
    for (let row = rect.top; row <= rect.bottom; row++) {
      const id = draftCellId(draft, sel.gridId, row, rect.left);
      sources.push(id ? draftRawWithoutId(draft, id) : "");
    }
    for (let row = rect.top; row <= rect.bottom; row++) {
      for (let col = rect.left + 1; col <= rect.right; col++) {
        const target = draftMaterializeCell(draft, sel.gridId, row, col);
        // writeCellVisible (not bare setRaw) so the target's own hidden id:: survives
        // and no ((ref)) pointing at it is orphaned — same as the fill-down branch.
        if (!target || !draftWriteVisible(draft, target, sources[row - rect.top])) return null;
      }
    }
    return true;
  }, () => afterApply?.());
  return result ?? false;
}

function moveWholeRows(
  gridId: string,
  rect: SheetRect,
  dir: "up" | "down",
  afterApply?: (rect: SheetRect) => void,
): SheetRect | null {
  const rows = gridRows(gridId);
  if (!rows) return null;
  if (dir === "up" && rect.top <= 0) return null;
  if (dir === "down" && rect.bottom >= rows.length - 1) return null;
  const count = rectRows(rect);
  const next = [...rows];
  const moving = next.splice(rect.top, count);
  const at = dir === "up" ? rect.top - 1 : rect.top + 1;
  next.splice(at, 0, ...moving);
  const moved = offsetRect(rect, dir);
  const result = plannedSheetMutation(gridId, "sheet:move-rows", (draft) =>
    draft.replaceChildren(gridId, next) ? moved : null, afterApply);
  return result;
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

function moveRectContent(
  gridId: string,
  rect: SheetRect,
  dir: SheetMoveDirection,
  afterApply?: (rect: SheetRect) => void,
): SheetRect | null {
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

  const moved = offsetRect(rect, dir);
  return plannedSheetMutation(gridId, "sheet:move-range", (draft) => {
    if (!draftEnsureRect(draft, gridId, materialize)) return null;

    if (dir === "left" || dir === "right") {
      for (let row = materialize.top; row <= materialize.bottom; row++) {
        const rowId = draftRows(draft, gridId)?.[row];
        if (!rowId) return null;
        const next = [...(draft.node(rowId)?.children ?? [])];
        rotateRowSegment(next, materialize.left, materialize.right, dir);
        if (!draft.replaceChildren(rowId, next)) return null;
      }
      return moved;
    }

    const nextByParent: Record<string, string[]> = {};
    const rowIds = draftRows(draft, gridId) ?? [];
    for (let row = materialize.top; row <= materialize.bottom; row++) {
      const rowId = rowIds[row];
      if (!rowId) return null;
      nextByParent[rowId] = [...(draft.node(rowId)?.children ?? [])];
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
    for (const [parent, children] of Object.entries(nextByParent)) {
      if (!draft.replaceChildren(parent, children)) return null;
    }
    return moved;
  }, afterApply);
}

export function moveSheetSelection(
  sel: SheetMutationSelection,
  dir: SheetMoveDirection,
  afterApply?: (selection: SheetMutationSelection) => void,
): SheetMutationSelection | null {
  const rect = rectForSheetSelection(sel);
  const bounds = sheetBounds(sel.gridId);
  if (sel.kind === "cell") {
    if (!cellIdAt(sel.gridId, sel.row, sel.col)) return null;
    const target = offsetPoint({ row: sel.row, col: sel.col }, dir);
    if (target.row < 0 || target.col < 0 || target.row >= bounds.rows || target.col >= bounds.cols) return null;
    if ((dir === "left" || dir === "right") && !cellIdAt(sel.gridId, target.row, target.col)) return null;
    const next = { kind: "cell", gridId: sel.gridId, row: target.row, col: target.col } as const;
    const moved = moveRectContent(sel.gridId, rect, dir, () => afterApply?.(next));
    return moved ? next : null;
  }

  if (
    (dir === "up" || dir === "down") &&
    rect.left === 0 &&
    rect.right === Math.max(0, bounds.cols - 1)
  ) {
    const delta = dir === "up" ? -1 : 1;
    const next: SheetMutationSelection = {
      kind: "range",
      gridId: sel.gridId,
      anchor: { row: sel.anchor.row + delta, col: sel.anchor.col },
      focus: { row: sel.focus.row + delta, col: sel.focus.col },
    };
    const movedRows = moveWholeRows(sel.gridId, rect, dir, () => afterApply?.(next));
    return movedRows ? next : null;
  }

  const delta = offsetPoint({ row: 0, col: 0 }, dir);
  const next: SheetMutationSelection = {
    kind: "range",
    gridId: sel.gridId,
    anchor: { row: sel.anchor.row + delta.row, col: sel.anchor.col + delta.col },
    focus: { row: sel.focus.row + delta.row, col: sel.focus.col + delta.col },
  };
  const moved = moveRectContent(sel.gridId, rect, dir, () => afterApply?.(next));
  return moved ? next : null;
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

function draftCellHasVisibleTextOrChildren(draft: PageMutationDraft, id: string): boolean {
  const node = draft.node(id);
  const text = visibleBody(node?.raw ?? "").join(" ").replace(/[\t\r\n]+/g, " ");
  return text.trim() !== "" || (node?.children.length ?? 0) > 0;
}

function pushPasteOverwriteToast(): void {
  pushToast("Pasted over existing cells.", "info", { action: { label: "Undo", run: () => undo() } });
}

export function splatStructuralSheetSelection(
  sel: SheetMutationSelection,
  text: string,
  afterApply?: (selection: SheetMutationSelection) => void,
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
  const selection: SheetMutationSelection = {
    kind: "range",
    gridId: sel.gridId,
    anchor,
    focus: { row: anchor.row + height - 1, col: anchor.col + width - 1 },
  };
  const result = plannedSheetMutation(sel.gridId, "sheet:paste-splat", (draft) => {
      if (!draftEnsureRows(draft, sel.gridId, anchor.row + height - 1)) return null;
      let overwroteNonEmpty = false;
      for (let r = 0; r < height; r++) {
        const row = rows[r];
        for (let c = 0; c < row.children.length; c++) {
          const srcCell = row.children[c];
          const target = draftMaterializeCell(draft, sel.gridId, anchor.row + r, anchor.col + c);
          if (!target) return null;
          if (draftCellHasVisibleTextOrChildren(draft, target)) overwroteNonEmpty = true;
          const existingChildren = [...(draft.node(target)?.children ?? [])];
          for (const child of existingChildren) if (!draft.deleteSubtree(child)) return null;
          if (!draftWriteVisible(draft, target, srcCell.raw)) return null;
          if (srcCell.children.length && !draft.insertOutlineChildren(target, srcCell.children)) return null;
        }
      }
      return { overwroteNonEmpty };
    }, (applied) => {
      if (applied.overwroteNonEmpty) pushPasteOverwriteToast();
      afterApply?.(selection);
    });
  if (!result) return null;
  return selection;
}

export function pasteTextIntoSheetSelection(
  sel: SheetMutationSelection,
  text: string,
  afterApply?: (selection: SheetMutationSelection) => void,
): SheetMutationSelection | null {
  const rect = rectForSheetSelection(sel);
  const anchor = { row: rect.top, col: rect.left };
  if (looksLikeDelimitedText(text)) {
    const matrix = parseDelimitedText(text);
    if (!matrix.length) return sel;
    const height = matrix.length;
    const width = Math.max(1, ...matrix.map((row) => row.length));
    const selection: SheetMutationSelection = height === 1 && width === 1
      ? { kind: "cell", gridId: sel.gridId, row: anchor.row, col: anchor.col }
      : {
          kind: "range",
          gridId: sel.gridId,
          anchor,
          focus: { row: anchor.row + height - 1, col: anchor.col + width - 1 },
        };
    const result = plannedSheetMutation(sel.gridId, "sheet:paste-matrix", (draft) => {
      if (!draftEnsureRows(draft, sel.gridId, anchor.row + matrix.length - 1)) return null;
      let overwroteNonEmpty = false;
      for (let r = 0; r < matrix.length; r++) {
        const row = matrix[r];
        for (let c = 0; c < row.length; c++) {
          const existing = draftCellId(draft, sel.gridId, anchor.row + r, anchor.col + c);
          if (existing && draftCellHasVisibleTextOrChildren(draft, existing)) overwroteNonEmpty = true;
          const id = draftMaterializeCell(draft, sel.gridId, anchor.row + r, anchor.col + c);
          if (!id || !draftWriteVisible(draft, id, row[c])) return null;
        }
      }
      return { overwroteNonEmpty };
    }, (applied) => {
      if (applied.overwroteNonEmpty) pushPasteOverwriteToast();
      afterApply?.(selection);
    });
    if (!result) return null;
    return selection;
  }

  if (looksIndentedOutline(text)) {
    const nodes = parseOutline(text);
    if (!nodes.length) return sel;
    const selection: SheetMutationSelection = { kind: "cell", gridId: sel.gridId, row: anchor.row, col: anchor.col };
    const ok = plannedSheetMutation(sel.gridId, "sheet:paste-outline", (draft) => {
      const id = draftMaterializeCell(draft, sel.gridId, anchor.row, anchor.col);
      return id && draft.insertOutlineChildren(id, nodes) ? true : null;
    }, () => afterApply?.(selection));
    return ok ? selection : null;
  }

  const selection: SheetMutationSelection = { kind: "cell", gridId: sel.gridId, row: anchor.row, col: anchor.col };
  const ok = plannedSheetMutation(sel.gridId, "sheet:paste-text", (draft) => {
    const id = draftMaterializeCell(draft, sel.gridId, anchor.row, anchor.col);
    return id && draftWriteVisible(draft, id, text.replace(/\r\n/g, "\n").replace(/\r/g, "\n"))
      ? true
      : null;
  }, () => afterApply?.(selection));
  return ok ? selection : null;
}
