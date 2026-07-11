import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, untrack, useContext, type JSX } from "solid-js";
import { observeNear, unobserveNear } from "../lazyObserve";
import { blockPageReadOnly, doc, formatForBlock, formatForPage, pageByName, readPageProperty } from "../store";
import { facetsFromDto, facetsOf, type Facets } from "../render/facets";
import { pageProperties, visibleBody, isRenderHiddenProp } from "../render/block";
import { InlineText } from "../render/inline";
import { editingId, editingOwner } from "../editorController";
import { SheetCellContext, type SheetCellCtx } from "../sheet/context";
import {
  cellOwner,
  cellSel,
  cellSurfaceKey,
  addEmptyTagColumn,
  clearSelectedSheetInstance,
  clearEmptyTagColumns,
  emptyTagColumnsForBoard,
  pruneEmptyTagColumns,
  registerSheetViewAdapter,
  rebaseSelectedCell,
  rebaseSelectedRange,
  setCellSel,
  startCellEditing,
  type CellSel,
} from "../sheet/selection";
import {
  boardGroupByOptions,
  fieldIdsForBlocks,
  cycleField,
  fieldLabel,
  groupKeysForBlock,
  isFieldId,
  isFormulaField,
  readField,
  writeTagDelta,
  writeField,
  type FieldId,
} from "../sheet/fields";
import { parseFields, sheetConfig, type FieldSpec } from "../sheet/config";
import { formulasOf, mergeFormulas } from "../sheet/formulaFields";
import { createFormulaFilterMemo, formulaRowKey, liveFormulaRowNode, type FormulaEvalRow } from "../sheet/formulaEval";
import { setBoardGroupBy } from "../sheet/mutations";
import { MARKERS } from "../markers";
import { graphEpoch, openDatePicker, openSheetCellContextMenu, openSheetContextMenu, pushToast, workflow } from "../ui";
import { blockBackgroundColor } from "../blockColors";
import type { RefGroup } from "../types";
import { Editor, SurfaceContext } from "./Block";
import { isBareTagName } from "../tags";
import { hydrateVisibleQueryPages, SHEET_RENDER_PAGE } from "../sheet/queryHydration";

interface RowRecord extends FormulaEvalRow {}

interface BoardColumn {
  key: string | null;
  label: string;
  rows: RowRecord[];
}

interface BoardDragCoordinator {
  start(pointerId: number, cancel: () => void): void;
  owns(pointerId: number, cancel: () => void): boolean;
  finish(pointerId: number, cancel: () => void): void;
  targetAtPoint(x: number, y: number): { col: number; columnId: string } | null;
}

let activeBoardDrag: { board: symbol; pointerId: number; cancel: () => void } | null = null;

function boardColumnId(key: string | null): string {
  return JSON.stringify(key);
}

const NONE_LABEL = "(none)";

export const __sheetBoardTestHooks: {
  onGroupingRowWalk?: (rowId: string) => void;
  onPointIndexRow?: (rowId: string) => void;
} = {};

export function SheetBoard(props: {
  ownerId: string;
  rowSource: "children" | "query";
  groupBy?: string | null;
  groups?: readonly RefGroup[];
  schemaPage?: string;
}): JSX.Element {
  const surfaceId = useContext(SurfaceContext);
  const groupBy = createMemo<FieldId>(() => {
    const raw = props.groupBy || "state";
    const normalized = raw.startsWith("formula.") ? `formula:${raw.slice("formula.".length)}` : raw;
    return isFieldId(normalized) ? normalized : "state";
  });
  const groupByOptions = createMemo<FieldId[]>(() => {
    const options = boardGroupByOptions(props.ownerId);
    const current = groupBy();
    return options.includes(current) ? options : [...options, current];
  });
  const [drag, setDrag] = createSignal<{ id: string; col: number; row: number; overCol: number | null } | null>(null);
  let boardElement: HTMLDivElement | undefined;
  const boardInstance = Symbol("sheet-board");
  const dragCoordinator: BoardDragCoordinator = {
    start(pointerId, cancel) {
      activeBoardDrag?.cancel();
      activeBoardDrag = { board: boardInstance, pointerId, cancel };
    },
    owns(pointerId, cancel) {
      return activeBoardDrag?.board === boardInstance && activeBoardDrag.pointerId === pointerId && activeBoardDrag.cancel === cancel;
    },
    finish(pointerId, cancel) {
      if (this.owns(pointerId, cancel)) activeBoardDrag = null;
    },
    targetAtPoint(x, y) {
      const column = (document.elementFromPoint(x, y) as HTMLElement | null)
        ?.closest(".sheet-board-column") as HTMLElement | null;
      if (!column || column.closest(".sheet-board") !== boardElement) return null;
      const col = Number(column.dataset.boardCol);
      const columnId = column.dataset.boardColumnId;
      return Number.isFinite(col) && columnId !== undefined ? { col, columnId } : null;
    },
  };
  onCleanup(() => {
    if (activeBoardDrag?.board === boardInstance) activeBoardDrag.cancel();
  });
  const [addingTag, setAddingTag] = createSignal(false);
  const [tagInputInvalid, setTagInputInvalid] = createSignal(false);
  const config = createMemo(() => {
    const owner = doc.byId[props.ownerId];
    return sheetConfig(owner ? facetsOf(owner.raw, formatForBlock(props.ownerId)).properties : []);
  });
  const schemaFields = createMemo<readonly FieldSpec[]>(() => {
    const own = config().fields;
    if (own.length > 0) return own;
    return props.schemaPage ? parseFields(readPageProperty(props.schemaPage, "tine.fields") ?? "") : [];
  });
  const pageFormulas = createMemo<ReadonlyMap<string, string>>(() => {
    if (!props.schemaPage) return new Map();
    const page = pageByName(props.schemaPage);
    return page ? formulasOf(pageProperties(page.preBlock, page.format)) : new Map();
  });
  const blockFormulas = createMemo<ReadonlyMap<string, string>>(() => {
    const owner = doc.byId[props.ownerId];
    return owner ? formulasOf(facetsOf(owner.raw, formatForBlock(props.ownerId)).properties) : new Map();
  });
  const formulas = createMemo(() => mergeFormulas(pageFormulas(), blockFormulas()));

  const allRows = createMemo<RowRecord[]>(() => {
    if (props.rowSource === "children") {
      return (doc.byId[props.ownerId]?.children ?? []).map((id) => ({
        id,
        page: doc.byId[id]?.page ?? doc.byId[props.ownerId]?.page ?? "",
      }));
    }
    return (props.groups ?? []).flatMap((g) => g.blocks.map((b) => ({ id: b.id, page: g.page, kind: g.kind, dto: b })));
  });
  const filterState = createFormulaFilterMemo({
    rows: allRows,
    formulas,
    filter: () => config().filter,
    ownerId: props.ownerId,
  });
  const rows = createMemo<RowRecord[]>(() => [...filterState().rows]);
  const filterError = () => filterState().error;
  const queryRowsVersion = createMemo(() => (props.rowSource === "query" ? props.groups : null));
  const [pendingEjection, setPendingEjection] =
    createSignal<{ id: string; groups: readonly RefGroup[] | undefined } | null>(null);

  createEffect(() => {
    const pending = pendingEjection();
    if (!pending) return;
    if (queryRowsVersion() === pending.groups) return;
    if (!allRows().some((row) => row.id === pending.id)) {
      pushToast("Moved out of this query's results", "info");
    }
    setPendingEjection(null);
  });

  const baseColumns = createMemo<BoardColumn[]>(() => {
    const now = new Date();
    return buildColumns(rows(), groupBy(), schemaFields(), { formulas: formulas(), now });
  });
  const columns = createMemo<BoardColumn[]>(() => {
    const cols = baseColumns();
    if (groupBy() !== "tags") return cols;
    const existing = new Set(cols.map((col) => col.key));
    const empty = emptyTagColumnsForBoard(props.ownerId)
      .filter((tag) => !existing.has(tag))
      .map((tag): BoardColumn => ({ key: tag, label: tag, rows: [] }));
    return [...cols, ...empty];
  });
  const dragVersion = createMemo(() => ({ groups: props.groups, groupBy: groupBy(), columns: columns() }));
  const pointIndex = createMemo(() => {
    const membership = new Map<string, { row: number; col: number }>();
    const first = new Map<string, { row: number; col: number }>();
    const rowOrder = new Map<string, number>();
    const byBlockId = new Map<string, { row: number; col: number; rowKey: string; columnId: string } | null>();
    rows().forEach((candidate, index) => rowOrder.set(formulaRowKey(candidate), index));
    columns().forEach((column, col) => {
      const columnId = boardColumnId(column.key);
      column.rows.forEach((candidate, row) => {
        __sheetBoardTestHooks.onPointIndexRow?.(candidate.id);
        const point = { row, col };
        const rowKey = formulaRowKey(candidate);
        membership.set(JSON.stringify([rowKey, columnId]), point);
        if (!first.has(rowKey)) first.set(rowKey, point);
        if (liveFormulaRowNode(candidate)) {
          if (byBlockId.has(candidate.id)) byBlockId.set(candidate.id, null);
          else byBlockId.set(candidate.id, { ...point, rowKey, columnId });
        }
      });
    });
    return { membership, first, rowOrder, byBlockId };
  });
  const pointForRowId = (rowId: string, preferredColumnId?: string) => {
    if (preferredColumnId !== undefined) {
      const point = pointIndex().membership.get(JSON.stringify([rowId, preferredColumnId]));
      if (point) return point;
      if (groupBy() === "tags") return null;
    }
    return pointIndex().first.get(rowId) ?? null;
  };
  createEffect(() => {
    const sel = cellSel();
    if (!sel || sel.gridId !== props.ownerId || sel.surfaceId !== surfaceId) return;
    if (sel.kind === "cell" && sel.rowId) {
      const point = pointForRowId(sel.rowId, sel.columnId);
      if (point) rebaseSelectedCell(props.ownerId, surfaceId, sel.rowId, point, boardColumnId(columns()[point.col].key));
      else clearSelectedSheetInstance(props.ownerId, surfaceId);
    } else if (sel.kind === "range" && sel.anchorRowId && sel.focusRowId) {
      const anchor = pointForRowId(sel.anchorRowId, sel.anchorColumnId);
      const focus = pointForRowId(sel.focusRowId, sel.focusColumnId);
      if (anchor && focus) {
        rebaseSelectedRange(
          props.ownerId,
          surfaceId,
          sel.anchorRowId,
          anchor,
          sel.focusRowId,
          focus,
          boardColumnId(columns()[anchor.col].key),
          boardColumnId(columns()[focus.col].key)
        );
      } else clearSelectedSheetInstance(props.ownerId, surfaceId);
    }
  });
  const [renderLimit, setRenderLimit] = createSignal(SHEET_RENDER_PAGE);
  createEffect(() => {
    props.groups;
    setRenderLimit(SHEET_RENDER_PAGE);
  });
  const displayedRows = createMemo(() => rows().slice(0, renderLimit()));
  const displayedIds = createMemo(() => new Set(displayedRows().map(formulaRowKey)));
  const displayedColumns = createMemo(() => {
    const ids = displayedIds();
    return columns().map((column) => ({ ...column, rows: column.rows.filter((row) => ids.has(formulaRowKey(row))) }));
  });
  const ensureDisplayedThrough = (row: number) => {
    if (row < 0 || row < displayedRows().length) return;
    setRenderLimit(Math.min(rows().length, Math.ceil((row + 1) / SHEET_RENDER_PAGE) * SHEET_RENDER_PAGE));
  };
  createEffect(() => {
    const sel = cellSel();
    if (!sel || sel.gridId !== props.ownerId || sel.surfaceId !== surfaceId) return;
    if (sel.kind === "cell" && sel.rowId) {
      const row = pointIndex().rowOrder.get(sel.rowId);
      if (row !== undefined) ensureDisplayedThrough(row);
    } else if (sel.kind === "range" && sel.anchorRowId && sel.focusRowId) {
      const anchor = pointIndex().rowOrder.get(sel.anchorRowId);
      const focus = pointIndex().rowOrder.get(sel.focusRowId);
      if (anchor !== undefined && focus !== undefined) ensureDisplayedThrough(Math.max(anchor, focus));
    }
  });
  createEffect(() => {
    if (groupBy() !== "tags") return;
    pruneEmptyTagColumns(
      props.ownerId,
      new Set(baseColumns().map((col) => col.key).filter((key): key is string => key !== null))
    );
  });
  createEffect(() => {
    graphEpoch();
    untrack(() => clearEmptyTagColumns(props.ownerId));
  });
  const maxRows = createMemo(() => Math.max(1, ...displayedColumns().map((c) => c.rows.length)));
  const formulaHintFields = createMemo(() => {
    const observed = observedFieldsForRows(rows(), props.rowSource === "query");
    const out: string[] = [];
    const seen = new Set<string>();
    for (const field of [...schemaFields().map((s) => s.field), ...observed, groupBy()]) {
      const name = formulaReferenceName(field);
      if (!name || seen.has(name)) continue;
      seen.add(name);
      out.push(name);
    }
    return out;
  });
  const formulaEntries = () => [...formulas().entries()];

  const selected = (col: number, row: number) => {
    const sel = cellSel();
    if (!sel || sel.gridId !== props.ownerId || (sel.surfaceId && sel.surfaceId !== surfaceId)) return false;
    if (sel.kind === "cell") return sel.col === col && sel.row === row;
    if (sel.kind === "range") return sel.focus.col === col && sel.focus.row === row;
    return false;
  };

  const noteQueryMove = (row: RowRecord) => {
    if (props.rowSource !== "query") return;
    setPendingEjection({ id: row.id, groups: props.groups });
  };

  const moveCard = (sel: CellSel, dir: "left" | "right"): boolean => {
    const cols = columns();
    if (sel.rowId && !displayedIds().has(sel.rowId)) return true;
    const point = sel.rowId ? pointForRowId(sel.rowId, sel.columnId) : { row: sel.row, col: sel.col };
    if (!point) return true;
    const from = cols[point.col];
    const row = sel.rowId ? from?.rows[point.row] : displayedColumns()[point.col]?.rows[point.row];
    if (!sel.rowId && !row) {
      clearSelectedSheetInstance(props.ownerId, surfaceId);
      return true;
    }
    const targetCol = dir === "left" ? point.col - 1 : point.col + 1;
    const target = cols[targetCol];
    if (isFormulaField(groupBy())) return true;
    if (!row || !target || !liveFormulaRowNode(row)) return true;
    if (moveRowToColumn(row, from.key, target.key, groupBy())) {
      noteQueryMove(row);
      const nextCols = columns();
      const nextCol = Math.max(0, nextCols.findIndex((c) => c.key === target.key));
      const nextRows = nextCols[nextCol]?.rows ?? [];
      setCellSel({
        gridId: props.ownerId,
        surfaceId,
        rowId: formulaRowKey(row),
        columnId: boardColumnId(nextCols[nextCol]?.key ?? null),
        col: nextCol,
        row: Math.max(0, nextRows.findIndex((r) => r.id === row.id)),
      });
    }
    return true;
  };

  onMount(() => {
    const dispose = registerSheetViewAdapter(props.ownerId, {
      bounds: () => ({ rows: maxRows(), cols: columns().length }),
      rowIdAt: (row, col = 0) => {
        const candidate = displayedColumns()[col]?.rows[row];
        return candidate ? formulaRowKey(candidate) : null;
      },
      columnIdAt: (_row, col) => columns()[col] ? boardColumnId(columns()[col].key) : null,
      blockIdAt: (row, col, rowId, columnId) => {
        if (rowId) {
          if (!displayedIds().has(rowId)) return null;
          const point = pointForRowId(rowId, columnId);
          const candidate = point ? columns()[point.col]?.rows[point.row] : null;
          return candidate && liveFormulaRowNode(candidate) ? candidate.id : null;
        }
        const candidate = displayedColumns()[col]?.rows[row];
        return candidate && liveFormulaRowNode(candidate) ? candidate.id : null;
      },
      cellForBlock: (blockId) => {
        const match = pointIndex().byBlockId.get(blockId);
        return match && displayedIds().has(match.rowKey)
          ? { kind: "cell", gridId: props.ownerId, surfaceId, rowId: match.rowKey, columnId: match.columnId, row: match.row, col: match.col }
          : null;
      },
      activate: (sel) => {
        if (sel.rowId && !displayedIds().has(sel.rowId)) return true;
        const point = sel.rowId ? pointForRowId(sel.rowId, sel.columnId) : { row: sel.row, col: sel.col };
        const row = sel.rowId
          ? (point ? columns()[point.col]?.rows[point.row] : null)
          : displayedColumns()[sel.col]?.rows[sel.row];
        if (!row || !liveFormulaRowNode(row)) return true;
        return false;
      },
      overtype: () => true,
      moveWithMod: (sel, dir) => {
        if (dir === "left" || dir === "right") return moveCard(sel, dir);
        return true;
      },
    }, surfaceId);
    onCleanup(dispose);
  });

  const dropCard = (
    row: RowRecord,
    rowKey: string,
    sourceColumnId: string,
    targetColumnId: string | null,
    dragGroupBy: FieldId,
  ) => {
    if (dragGroupBy !== groupBy() || isFormulaField(groupBy()) || !displayedIds().has(rowKey)) return;
    const cols = columns();
    const sourcePoint = pointForRowId(rowKey, sourceColumnId);
    const source = sourcePoint ? cols[sourcePoint.col] : null;
    const target = targetColumnId === null
      ? null
      : cols.find((column) => boardColumnId(column.key) === targetColumnId) ?? null;
    const currentRow = sourcePoint ? source?.rows[sourcePoint.row] : null;
    if (!source || !target || sourceColumnId === targetColumnId ||
        boardColumnId(source.key) !== sourceColumnId || !currentRow ||
        formulaRowKey(currentRow) !== rowKey || formulaRowKey(row) !== rowKey ||
        !liveFormulaRowNode(currentRow)) return;
    if (moveRowToColumn(currentRow, source.key, target.key, groupBy())) noteQueryMove(currentRow);
  };

  const openSheetMenu = (e: MouseEvent) => {
    if (!doc.byId[props.ownerId]) return;
    e.preventDefault();
    e.stopPropagation();
    openSheetContextMenu(e.clientX, e.clientY, props.ownerId, "board", props.rowSource, groupBy(), {
      schemaPage: props.schemaPage,
      fields: formulaHintFields(),
      formulas: formulaEntries(),
      filter: config().filter,
    });
  };

  const commitNewTagColumn = (value: string): boolean => {
    const tag = value.trim();
    const exists = columns().some((col) => col.key === tag);
    if (!isBareTagName(tag) || exists) {
      setTagInputInvalid(true);
      if (tag && !exists) pushToast("Invalid tag name", "warn");
      return false;
    }
    addEmptyTagColumn(props.ownerId, tag);
    setAddingTag(false);
    setTagInputInvalid(false);
    return true;
  };

  return (
    <div class="sheet-board-wrap">
      <Show when={!blockPageReadOnly(props.ownerId)}>
        <div class="sheet-board-toolbar">
          <span>Group by</span>
          <select
            class="sheet-board-groupby"
            value={groupBy()}
            aria-label="Group by"
            onPointerDown={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.stopPropagation()}
            onChange={(e) => setBoardGroupBy(props.ownerId, e.currentTarget.value as FieldId)}
          >
            <For each={groupByOptions()}>
              {(field) => <option value={field}>{fieldLabel(field)}</option>}
            </For>
          </select>
        </div>
      </Show>
      <Show when={columns().length > 0} fallback={<div class="sheet-board sheet-empty">empty board</div>}>
        <div
          ref={boardElement}
          class="sheet-board"
          classList={{ "sheet-filter-broken": !!filterError() }}
          data-sheet-grid-id={props.ownerId}
          data-sheet-surface-id={surfaceId}
          onContextMenu={openSheetMenu}
        >
          <Show when={filterError()}>
            {(err) => (
              <span class="sheet-filter-error" title={err()}>
                Filter disabled
              </span>
            )}
          </Show>
          <For each={displayedColumns()}>
            {(col, colIndex) => (
              <section
                class="sheet-board-column"
                classList={{ "sheet-board-drop": drag()?.overCol === colIndex() }}
                data-board-col={colIndex()}
                data-board-column-id={boardColumnId(col.key)}
              >
                <header class="sheet-board-header">
                  <span>{col.label}</span>
                  <span class="sheet-board-count">{col.rows.length}</span>
                </header>
                <div class="sheet-board-cards">
                  <For each={col.rows}>
                    {(row, rowIndex) => (
                      <BoardCard
                        ownerId={props.ownerId}
                        surfaceId={surfaceId}
                        row={row}
                        groupBy={groupBy()}
                        columnId={boardColumnId(col.key)}
                        colIndex={colIndex()}
                        rowIndex={rowIndex()}
                        selected={selected(colIndex(), rowIndex())}
                        dragging={drag()?.id === row.id && drag()?.col === colIndex() && drag()?.row === rowIndex()}
                        canMove={!isFormulaField(groupBy())}
                        dragVersion={dragVersion()}
                        dragCoordinator={dragCoordinator}
                        hydrate={props.rowSource === "query"
                          ? () => void hydrateVisibleQueryPages([row], props.groups)
                          : undefined}
                        setDrag={setDrag}
                        dropCard={dropCard}
                      />
                    )}
                  </For>
                </div>
              </section>
            )}
          </For>
          <Show when={groupBy() === "tags"}>
            <section class="sheet-board-column sheet-board-add-tag-column">
              <Show
                when={addingTag()}
                fallback={
                  <button
                    class="sheet-board-add-tag-ghost"
                    title="Add tag column"
                    onClick={(e) => {
                      e.stopPropagation();
                      setAddingTag(true);
                      setTagInputInvalid(false);
                    }}
                  >
                    <span class="sheet-ghost-plus">+</span>
                    <span>new tag</span>
                  </button>
                }
              >
                <input
                  class="sheet-board-add-tag-input"
                  classList={{ "sheet-input-invalid": tagInputInvalid() }}
                  autofocus
                  placeholder="new-tag"
                  onPointerDown={(e) => e.stopPropagation()}
                  onMouseDown={(e) => e.stopPropagation()}
                  onInput={() => setTagInputInvalid(false)}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === "Enter") commitNewTagColumn(e.currentTarget.value);
                    else if (e.key === "Escape") {
                      setAddingTag(false);
                      setTagInputInvalid(false);
                    }
                  }}
                  onBlur={() => {
                    setAddingTag(false);
                    setTagInputInvalid(false);
                  }}
                />
              </Show>
            </section>
          </Show>
        </div>
      </Show>
      <Show when={displayedRows().length < rows().length}>
        <button
          class="sheet-add-row-ghost sheet-load-more"
          onClick={(e) => {
            e.stopPropagation();
            setRenderLimit((limit) => limit + SHEET_RENDER_PAGE);
          }}
        >
          Load {Math.min(SHEET_RENDER_PAGE, rows().length - displayedRows().length)} more cards
          ({displayedRows().length} of {rows().length})
        </button>
      </Show>
    </div>
  );
}

function buildColumns(
  rows: readonly RowRecord[],
  groupBy: FieldId,
  schema: readonly FieldSpec[] = [],
  opts: { formulas?: ReadonlyMap<string, string>; now?: Date } = {}
): BoardColumn[] {
  const rowsByKey = new Map<string | null, RowRecord[]>();
  const keys: (string | null)[] = [];
  const allKeys: (string | null)[] = [];
  const seenAllKeys = new Set<string | null>();
  let hasNull = false;
  let hasFormulaError = false;
  for (const row of rows) {
    __sheetBoardTestHooks.onGroupingRowWalk?.(row.id);
    const rowKeys = groupKeysForBlock(row, groupBy, opts);
    keys.push(rowKeys[0] ?? null);
    const seenForRow = new Set<string | null>();
    for (const key of rowKeys) {
      hasNull ||= key === null;
      hasFormulaError ||= key === "(error)";
      if (!seenAllKeys.has(key)) {
        seenAllKeys.add(key);
        allKeys.push(key);
      }
      if (seenForRow.has(key)) continue;
      seenForRow.add(key);
      const bucket = rowsByKey.get(key);
      if (bucket) bucket.push(row);
      else rowsByKey.set(key, [row]);
    }
  }
  let order: (string | null)[];
  const enumValues = enumValuesFor(schema, groupBy);
  if (isFormulaField(groupBy)) {
    const present = new Set(keys.filter((key): key is string => key !== null));
    const booleanish = present.has("true") || present.has("false");
    order = [];
    if (booleanish) {
      if (present.has("true")) order.push("true");
      if (present.has("false")) order.push("false");
    }
    for (const key of keys) {
      if (key === null || key === "(error)") continue;
      if (booleanish && (key === "true" || key === "false")) continue;
      if (!order.includes(key)) order.push(key);
    }
  } else if (groupBy === "tags") {
    order = [];
    for (const key of allKeys) if (key !== null) order.push(key);
  } else if (enumValues) {
    order = [...enumValues];
    for (const key of keys) if (key !== null && !order.includes(key)) order.push(key);
    order.push(null);
  } else if (groupBy === "state") {
    const standard = workflow() === "todo" ? ["TODO", "DOING", "DONE"] : ["LATER", "NOW", "DONE"];
    order = [
      ...standard,
      ...MARKERS.filter((m) => !standard.includes(m) && keys.includes(m)),
    ];
  } else if (groupBy === "priority") {
    order = ["A", "B", "C"];
  } else {
    order = [];
    for (const key of keys) if (key !== null && !order.includes(key)) order.push(key);
  }
  if (hasNull && !order.includes(null)) order.push(null);
  if (isFormulaField(groupBy) && hasFormulaError && !order.includes("(error)")) {
    order.push("(error)");
  }
  if (order.length === 0) order = [null];
  return order.map((key) => ({
    key,
    label: key === null ? NONE_LABEL : groupBy === "priority" ? `[#${key}]` : key,
    rows: rowsByKey.get(key) ?? [],
  }));
}

function enumValuesFor(schema: readonly FieldSpec[], field: FieldId): readonly string[] | null {
  const spec = schema.find((s) => s.field === field);
  return spec && typeof spec.type === "object" && "enum" in spec.type ? spec.type.enum : null;
}

function observedFieldsForRows(rows: readonly RowRecord[], includePage: boolean): FieldId[] {
  const loadedIds = rows.filter((r) => liveFormulaRowNode(r)).map((r) => r.id);
  if (loadedIds.length === rows.length) return fieldIdsForBlocks(loadedIds, { includePage });
  return fieldIdsForRecords(rows, includePage);
}

function fieldIdsForRecords(rows: readonly RowRecord[], includePage: boolean): FieldId[] {
  const out: FieldId[] = [];
  const props: FieldId[] = [];
  const seenProps = new Set<string>();
  let hasState = false;
  let hasPriority = false;
  let hasScheduled = false;
  let hasDeadline = false;
  let hasTags = false;
  for (const r of rows) {
    const f = recordFacets(r);
    if (!f) continue;
    hasState ||= !!f.marker;
    hasPriority ||= !!f.priority;
    hasScheduled ||= !!f.scheduled;
    hasDeadline ||= !!f.deadline;
    hasTags ||= f.tags.length > 0;
    for (const [key] of f.properties) {
      if (isRenderHiddenProp(key)) continue;
      const field: FieldId = `prop:${key}`;
      if (!seenProps.has(field)) {
        seenProps.add(field);
        props.push(field);
      }
    }
  }
  if (hasState) out.push("state");
  if (hasPriority) out.push("priority");
  if (hasScheduled) out.push("scheduled");
  if (hasDeadline) out.push("deadline");
  if (hasTags) out.push("tags");
  out.push(...props);
  if (includePage) out.push("page");
  return out;
}

function formulaReferenceName(field: FieldId): string | null {
  if (isFormulaField(field)) return null;
  if (field.startsWith("prop:")) return field.slice(5);
  return field;
}

function recordFacets(row: RowRecord): Facets | null {
  const n = liveFormulaRowNode(row);
  if (n) return facetsOf(n.raw, formatForBlock(row.id));
  return row.dto ? facetsFromDto(row.dto) : null;
}

function moveRowToColumn(row: RowRecord, from: string | null, target: string | null, field: FieldId): boolean {
  if (isFormulaField(field)) return false;
  if (field !== "tags") return writeField(row.id, field, target ?? "");
  const tags = groupKeysForBlock(row, "tags").filter((key): key is string => key !== null);
  if (from === null) return target !== null && writeTagDelta(row.id, { add: target });
  if (target === null) return tags.length === 1 && writeTagDelta(row.id, { remove: from });
  return writeTagDelta(row.id, { remove: from, add: target });
}

function dtoField(row: RowRecord, field: FieldId): string | null {
  if (isFormulaField(field)) return null;
  const f = recordFacets(row);
  if (!f) return null;
  if (field === "state") return f.marker;
  if (field === "priority") return f.priority;
  if (field === "scheduled") return f.scheduled;
  if (field === "deadline") return f.deadline;
  if (field === "tags") return f.tags.join(" ") || null;
  if (field === "page") return row.page;
  const key = field.slice(5);
  return f.properties.find(([k]) => k === key)?.[1] ?? null;
}

function rowRaw(row: RowRecord): string {
  return liveFormulaRowNode(row)?.raw ?? row.dto?.raw ?? "";
}

function rowTitle(row: RowRecord): string {
  return visibleBody(rowRaw(row))[0] ?? "";
}

// Lazy-mount virtualization (P2): a board card's heavy content (title
// InlineText parse, chips, hover handle) is deferred until the card first comes
// near the viewport, mirroring the table (SheetTable.tsx) and the block-body
// pattern ([[tine-block-virtualization]]). The <article> shell, its data-row/col
// attrs, selection class and drag handlers stay mounted for EVERY card so
// selection / keyboard nav / drag hit-testing keep working off-screen.
// Render-once-keep: a card rendered once (latched by block id) renders eagerly
// forever. Module-level, shared across surfaces, bounded by the working set.
const renderedBoardCards = new Set<string>();

export function resetBoardCardVirtualizationForTests() {
  renderedBoardCards.clear();
}

function BoardCard(props: {
  ownerId: string;
  surfaceId: string;
  row: RowRecord;
  groupBy: FieldId;
  columnId: string;
  colIndex: number;
  rowIndex: number;
  selected: boolean;
  dragging: boolean;
  canMove: boolean;
  dragVersion: unknown;
  dragCoordinator: BoardDragCoordinator;
  hydrate?: () => void;
  setDrag: (v: { id: string; col: number; row: number; overCol: number | null } | null) => void;
  dropCard: (row: RowRecord, rowKey: string, sourceColumnId: string, targetColumnId: string | null, groupBy: FieldId) => void;
}): JSX.Element {
  const cell = (): SheetCellCtx => ({
    gridId: props.ownerId,
    surfaceId: props.surfaceId,
    rowId: formulaRowKey(props.row),
    columnId: props.columnId,
    row: props.rowIndex,
    col: props.colIndex,
  });
  const editing = () => editingId() === props.row.id && editingOwner() === cellOwner(cell());
  const fmt = () => (liveFormulaRowNode(props.row) ? formatForBlock(props.row.id) : formatForPage(props.row.page));
  const [near, setNear] = createSignal(renderedBoardCards.has(props.row.id));
  const observeCard = (el: Element) => {
    if (near()) {
      props.hydrate?.();
      return;
    }
    observeNear(el, () => {
      renderedBoardCards.add(props.row.id);
      setNear(true);
      props.hydrate?.();
    });
    onCleanup(() => unobserveNear(el));
  };
  const bgColor = createMemo(() => {
    const f = recordFacets(props.row);
    return f ? blockBackgroundColor(f.properties) : undefined;
  });

  const select = () => setCellSel(cell());
  let cancelActiveDrag: (() => void) | null = null;
  let observedDragVersion = props.dragVersion;
  createEffect(() => {
    const next = props.dragVersion;
    if (next === observedDragVersion) return;
    observedDragVersion = next;
    cancelActiveDrag?.();
  });
  onCleanup(() => cancelActiveDrag?.());

  const beginPointerDrag = (e: PointerEvent) => {
    if (!props.canMove) return;
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
    cancelActiveDrag?.();
    const card = e.currentTarget as HTMLElement;
    if (typeof card.setPointerCapture === "function" && typeof e.pointerId === "number") {
      card.setPointerCapture(e.pointerId);
    }
    const startX = e.clientX;
    const startY = e.clientY;
    const pointerId = e.pointerId;
    let moved = false;
    let overCol: number | null = null;
    let targetColumnId: string | null = null;
    let ghost: HTMLElement | null = null;
    let finished = false;
    const rowKey = formulaRowKey(props.row);
    const sourceColumnId = props.columnId;
    const dragGroupBy = props.groupBy;
    const dragState = () => ({ id: props.row.id, col: props.colIndex, row: props.rowIndex, overCol });
    const updateGhost = (ev: PointerEvent) => {
      if (ghost) ghost.style.transform = `translate(${Math.round(ev.clientX + 10)}px, ${Math.round(ev.clientY + 10)}px)`;
    };
    const createGhost = (ev: PointerEvent) => {
      const rect = card.getBoundingClientRect();
      ghost = card.cloneNode(true) as HTMLElement;
      ghost.classList.add("sheet-board-drag-ghost");
      ghost.classList.remove("sheet-cell-selected", "sheet-board-card-dragging");
      ghost.setAttribute("aria-hidden", "true");
      ghost.style.width = `${rect.width}px`;
      ghost.style.minHeight = `${rect.height}px`;
      document.body.appendChild(ghost);
      document.body.classList.add("sheet-board-dragging");
      updateGhost(ev);
      props.setDrag(dragState());
    };
    const cleanup = () => {
      ghost?.remove();
      ghost = null;
      if (!document.body.querySelector(".sheet-board-drag-ghost")) {
        document.body.classList.remove("sheet-board-dragging");
      }
      props.setDrag(null);
    };
    const removeListeners = () => {
      window.removeEventListener("pointermove", onMove, true);
      window.removeEventListener("pointerup", onUp, true);
      window.removeEventListener("pointercancel", onPointerCancel, true);
      window.removeEventListener("blur", onCancel, true);
      window.removeEventListener("keydown", onKeyDown, true);
      card.removeEventListener("lostpointercapture", onLostPointerCapture, true);
    };
    const onMove = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId || !props.dragCoordinator.owns(pointerId, onCancel)) return;
      if (!moved && Math.hypot(ev.clientX - startX, ev.clientY - startY) < 4) return;
      if (!moved) {
        moved = true;
        createGhost(ev);
      } else updateGhost(ev);
      const target = props.dragCoordinator.targetAtPoint(ev.clientX, ev.clientY);
      const nextOverCol = target?.col ?? null;
      const nextTargetColumnId = target?.columnId ?? null;
      if (nextOverCol !== overCol || nextTargetColumnId !== targetColumnId) {
        overCol = nextOverCol;
        targetColumnId = nextTargetColumnId;
        props.setDrag(dragState());
      }
      ev.preventDefault();
    };
    const onUp = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId || !props.dragCoordinator.owns(pointerId, onCancel)) return;
      const releaseTarget = moved ? props.dragCoordinator.targetAtPoint(ev.clientX, ev.clientY) : null;
      if (moved) {
        finish();
        if (releaseTarget) {
          props.dropCard(props.row, rowKey, sourceColumnId, releaseTarget.columnId, dragGroupBy);
        }
        ev.preventDefault();
      } else {
        finish();
      }
    };
    const finish = () => {
      if (finished) return;
      finished = true;
      removeListeners();
      // Escape/blur/supersession can finish before the browser's natural
      // pointerup release. Drop capture after detaching lostpointercapture so a
      // canceled pointer cannot keep retargeting later gestures to this card.
      try {
        if (typeof card.hasPointerCapture === "function" &&
            typeof card.releasePointerCapture === "function" &&
            card.hasPointerCapture(pointerId)) {
          card.releasePointerCapture(pointerId);
        }
      } catch {
        // Capture may already have been revoked by the platform or element removal.
      }
      cancelActiveDrag = null;
      props.dragCoordinator.finish(pointerId, onCancel);
      cleanup();
    };
    const onCancel = () => finish();
    const onPointerCancel = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId || !props.dragCoordinator.owns(pointerId, onCancel)) return;
      onCancel();
    };
    const onLostPointerCapture = (ev: PointerEvent) => {
      if (ev.pointerId !== pointerId || !props.dragCoordinator.owns(pointerId, onCancel)) return;
      onCancel();
    };
    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.key !== "Escape") return;
      ev.preventDefault();
      onCancel();
    };
    props.dragCoordinator.start(pointerId, onCancel);
    window.addEventListener("pointermove", onMove, true);
    window.addEventListener("pointerup", onUp, true);
    window.addEventListener("pointercancel", onPointerCancel, true);
    window.addEventListener("blur", onCancel, true);
    window.addEventListener("keydown", onKeyDown, true);
    card.addEventListener("lostpointercapture", onLostPointerCapture, true);
    cancelActiveDrag = onCancel;
  };

  const onClick = (e: MouseEvent) => {
    e.stopPropagation();
    select();
  };
  const onDoubleClick = (e: MouseEvent) => {
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey) return;
    e.preventDefault();
    e.stopPropagation();
    select();
    if (liveFormulaRowNode(props.row)) startCellEditing(cell());
  };
  const onChipClick = (field: FieldId, e: MouseEvent) => {
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey) return;
    e.preventDefault();
    e.stopPropagation();
    select();
    if (!liveFormulaRowNode(props.row)) return;
    if (field === "priority") cycleField(props.row.id, "priority");
    else if (field === "scheduled" || field === "deadline") {
      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
      openDatePicker(props.row.id, field, rect.left, rect.bottom + 4);
    }
  };
  const openCellMenu = (e: MouseEvent) => {
    if (!liveFormulaRowNode(props.row)) return;
    e.preventDefault();
    e.stopPropagation();
    select();
    openSheetCellContextMenu(e.clientX, e.clientY, props.row.id);
  };
  const openCellMenuFromHandle = (e: MouseEvent) => {
    if (!liveFormulaRowNode(props.row)) return;
    e.preventDefault();
    e.stopPropagation();
    select();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openSheetCellContextMenu(rect.right, rect.bottom + 2, props.row.id);
  };

  return (
    <article
      class="sheet-board-card"
      classList={{
        "sheet-cell-selected": props.selected,
        "sheet-board-card-dragging": props.dragging,
        "sheet-board-card-static": !props.canMove,
      }}
      data-sheet-grid-id={props.ownerId}
      data-sheet-surface-id={props.surfaceId}
      data-sheet-column-id={props.columnId}
      data-row={props.rowIndex}
      data-col={props.colIndex}
      data-block-id={props.row.id}
      onPointerDown={beginPointerDrag}
      onMouseDown={(e: MouseEvent) => {
        // The card lives inside the query block's content subtree; without this
        // the mousedown bubbles into that block's beginEditGesture and mouseup
        // starts an UNSCOPED edit of the {{query}} block, stomping the card's
        // scoped edit and clearing the cell selection (grid cells already stop
        // propagation in their own mousedown handler).
        e.stopPropagation();
      }}
      onClick={onClick}
      onDblClick={onDoubleClick}
      onContextMenu={openCellMenu}
      style={bgColor() ? { background: bgColor() } : undefined}
      ref={observeCard}
    >
      <Show when={near() && liveFormulaRowNode(props.row)}>
        <button
          class="sheet-cell-handle sheet-card-handle"
          title="Card menu"
          onPointerDown={(e) => {
            e.preventDefault();
            e.stopPropagation();
          }}
          onMouseDown={(e) => {
            e.preventDefault();
            e.stopPropagation();
          }}
          onClick={openCellMenuFromHandle}
        >
          ⋮
        </button>
      </Show>
      <Show
        when={editing()}
        fallback={
          <Show
            when={near()}
            fallback={<div class="sheet-board-card-title sheet-cell-defer">{rowTitle(props.row)}</div>}
          >
            <div class="sheet-board-card-title">
              <InlineText text={rowTitle(props.row)} format={fmt()} />
            </div>
            <CardChips row={props.row} groupBy={props.groupBy} onFieldClick={onChipClick} />
          </Show>
        }
      >
        <SheetCellContext.Provider value={cell()}>
          <SurfaceContext.Provider value={cellSurfaceKey(props.ownerId, props.surfaceId)}>
            <Editor id={props.row.id} />
          </SurfaceContext.Provider>
        </SheetCellContext.Provider>
      </Show>
    </article>
  );
}

function CardChips(props: { row: RowRecord; groupBy: FieldId; onFieldClick: (field: FieldId, e: MouseEvent) => void }): JSX.Element {
  const value = (field: FieldId) => (liveFormulaRowNode(props.row) ? readField(props.row.id, field)?.text ?? "" : dtoField(props.row, field) ?? "");
  const priority = () => props.groupBy === "priority" ? "" : value("priority");
  const scheduled = () => props.groupBy === "scheduled" ? "" : value("scheduled");
  const deadline = () => props.groupBy === "deadline" ? "" : value("deadline");
  const tags = () => props.groupBy === "tags" ? "" : value("tags");
  const stopDoubleClick = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
  };
  return (
    <div class="sheet-board-card-chips">
      <Show when={priority()}>
        <span
          class="block-priority"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => props.onFieldClick("priority", e)}
          onDblClick={stopDoubleClick}
        >
          {priority()}
        </span>
      </Show>
      <Show when={scheduled()}>
        <span
          class="date-chip scheduled"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => props.onFieldClick("scheduled", e)}
          onDblClick={stopDoubleClick}
        >
          {scheduled()}
        </span>
      </Show>
      <Show when={deadline()}>
        <span
          class="date-chip deadline"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => props.onFieldClick("deadline", e)}
          onDblClick={stopDoubleClick}
        >
          {deadline()}
        </span>
      </Show>
      <Show when={tags()}>
        <For each={tags().split(/\s+/).filter(Boolean)}>
          {(tag) => <span class="sheet-tag-chip">{tag.startsWith("#") ? tag : `#${tag}`}</span>}
        </For>
      </Show>
    </div>
  );
}
