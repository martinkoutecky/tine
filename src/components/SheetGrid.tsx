import { For, Match, Show, Switch, createEffect, createMemo, createSignal, onCleanup, useContext, type JSX } from "solid-js";
import { blockPageReadOnly, doc, formatForBlock } from "../store";
import { AstBody } from "../render/body";
import { visibleBody } from "../render/block";
import { facetsOf } from "../render/facets";
import { sheetConfig } from "../sheet/config";
import {
  buildMatrixWindow,
  noteMatrixWindowColumns,
  observeMatrixDimensions,
  type MatrixCell,
} from "../sheet/matrix";
import { collectAggregateColumns } from "../sheet/aggregate";
import { SheetCellContext, type SheetCellCtx } from "../sheet/context";
import {
  cellOwner,
  cellSel,
  cellSurfaceKey,
  aggregateFooterPinned,
  colSeamSel,
  growSheetEdge,
  rowSeamSel,
  registerSheetVisibilityHook,
  registerSheetViewAdapter,
  setCellSel,
  setAggregateFooterPinned,
  startCellEditing,
  toggleAggregateFooterPinned,
  type SheetSel,
} from "../sheet/selection";
import {
  beginCellPointerSelection,
  isSheetPointerInteractive,
  sheetGridIdFromEventTarget,
} from "../sheet/pointerSelection";
import { setColumnWidth } from "../sheet/mutations";
import { editorOffsetFromRenderedRange } from "../render/spans";
import { isSheetCellHidden, splitProps } from "../editor/properties";
import { forbidsEditEntry } from "../editor/editTargets";
import { editingId, editingOwner, startEditing } from "../editorController";
import { openSheetCellContextMenu, openSheetContextMenu } from "../ui";
import { blockBackgroundColor } from "../blockColors";
import { Editor, SurfaceContext } from "./Block";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { SheetAggregateCornerToggle, SheetAggregateFooterCell } from "./SheetAggregateFooter";
import { SheetContainerOverlayContext } from "./SheetContainerOverlay";

const MAX_GRID_DEPTH = 5;
export const GRID_RENDER_PAGE = 200;
export const GRID_RENDER_CELL_LIMIT = 2_000;

function configForBlock(id: string) {
  const node = doc.byId[id];
  return sheetConfig(node ? facetsOf(node.raw, formatForBlock(id)).properties : []);
}

function blockChildren(id: string): string[] {
  return doc.byId[id]?.children ?? [];
}

function columnTracks(cols: number, widths: ReadonlyMap<number, number>, preview?: { col: number; px: number }, start = 0): string {
  const tracks: string[] = [];
  for (let col = start; col < start + cols; col++) {
    const px = preview?.col === col ? preview.px : widths.get(col);
    tracks.push(px == null ? "max-content" : `${px}px`);
  }
  return tracks.join(" ");
}

function cellInGrid(grid: HTMLElement, row: number, col: number): HTMLElement | null {
  return grid.querySelector(`:scope > .sheet-cell[data-row="${row}"][data-col="${col}"]`) as HTMLElement | null;
}

function measuredColumnTracks(grid: HTMLElement, cols: number, start = 0): string | null {
  const tracks: string[] = [];
  for (let col = start; col < start + cols; col++) {
    const cell =
      cellInGrid(grid, 0, col) ??
      grid.querySelector(`:scope > .sheet-cell[data-col="${col}"]`) as HTMLElement | null;
    const width = cell?.getBoundingClientRect().width ?? 0;
    if (width <= 0) return null;
    tracks.push(`${Math.round(width)}px`);
  }
  return tracks.join(" ");
}

function seamStyleFor(grid: HTMLElement, sel: SheetSel, matrix: { rows: number; cols: number; rowStart: number; colStart: number }): JSX.CSSProperties | null {
  if ((sel.kind !== "row-seam" && sel.kind !== "col-seam") || matrix.rows <= 0 || matrix.cols <= 0) return null;
  const gridRect = grid.getBoundingClientRect();

  if (sel.kind === "row-seam") {
    const firstRow = matrix.rowStart;
    const lastRow = matrix.rowStart + matrix.rows - 1;
    const row = sel.at <= firstRow ? firstRow : sel.at > lastRow ? lastRow : sel.at;
    const col = Math.max(matrix.colStart, Math.min(sel.anchor.col, matrix.colStart + matrix.cols - 1));
    const cell = cellInGrid(grid, row, col);
    if (!cell) return null;
    const rect = cell.getBoundingClientRect();
    const y = sel.at <= firstRow ? rect.top - gridRect.top : sel.at > lastRow ? rect.bottom - gridRect.top : rect.top - gridRect.top;
    // Clamp into the visible content box: the outermost boundary seams would
    // otherwise land exactly on the overflow clip edge and paint nothing.
    const top = Math.max(0, Math.min(Math.round(y + grid.scrollTop - 1), grid.scrollHeight - 2));
    // Clamp the cross axis too: a bar whose right edge rounds 1px past
    // scrollWidth momentarily grows the content and flashes a scrollbar on the
    // subgrid (its overflow is `auto`, unlike the top level's clipped y-axis).
    const width = Math.round(rect.width);
    const left = Math.max(0, Math.min(Math.round(rect.left - gridRect.left + grid.scrollLeft), Math.max(0, grid.scrollWidth - width)));
    return {
      left: `${left}px`,
      top: `${top}px`,
      width: `${width}px`,
      height: "2px",
    };
  }

  const row = Math.max(matrix.rowStart, Math.min(sel.anchor.row, matrix.rowStart + matrix.rows - 1));
  const firstCol = matrix.colStart;
  const lastCol = matrix.colStart + matrix.cols - 1;
  const col = sel.at <= firstCol ? firstCol : sel.at > lastCol ? lastCol : sel.at;
  const cell = cellInGrid(grid, row, col);
  if (!cell) return null;
  const rect = cell.getBoundingClientRect();
  const x = sel.at <= firstCol ? rect.left - gridRect.left : sel.at > lastCol ? rect.right - gridRect.left : rect.left - gridRect.left;
  // Same boundary clamp as row seams (right-edge seam vs the overflow clip).
  const left = Math.max(0, Math.min(Math.round(x + grid.scrollLeft - 1), grid.scrollWidth - 2));
  const height = Math.round(rect.height);
  const top = Math.max(0, Math.min(Math.round(rect.top - gridRect.top + grid.scrollTop), Math.max(0, grid.scrollHeight - height)));
  return {
    left: `${left}px`,
    top: `${top}px`,
    width: "2px",
    height: `${height}px`,
  };
}

function trackEdges(grid: HTMLElement, rows: number, cols: number, rowStart = 0, colStart = 0): { rowEdges: number[]; colEdges: number[] } | null {
  if (rows <= 0 || cols <= 0) return null;
  const gridRect = grid.getBoundingClientRect();
  const colEdges: number[] = [];
  for (let at = 0; at <= cols; at++) {
    const col = colStart + at;
    if (at === 0) {
      const first = cellInGrid(grid, rowStart, colStart);
      if (!first) return null;
      colEdges.push(first.getBoundingClientRect().left - gridRect.left);
    } else if (at === cols) {
      const last = cellInGrid(grid, rowStart, colStart + cols - 1);
      if (!last) return null;
      colEdges.push(last.getBoundingClientRect().right - gridRect.left);
    } else {
      const prev = cellInGrid(grid, rowStart, col - 1);
      const next = cellInGrid(grid, rowStart, col);
      if (!prev || !next) return null;
      colEdges.push((prev.getBoundingClientRect().right + next.getBoundingClientRect().left) / 2 - gridRect.left);
    }
  }

  const rowEdges: number[] = [];
  for (let at = 0; at <= rows; at++) {
    const row = rowStart + at;
    if (at === 0) {
      const first = cellInGrid(grid, rowStart, colStart);
      if (!first) return null;
      rowEdges.push(first.getBoundingClientRect().top - gridRect.top);
    } else if (at === rows) {
      const last = cellInGrid(grid, rowStart + rows - 1, colStart);
      if (!last) return null;
      rowEdges.push(last.getBoundingClientRect().bottom - gridRect.top);
    } else {
      const prev = cellInGrid(grid, row - 1, colStart);
      const next = cellInGrid(grid, row, colStart);
      if (!prev || !next) return null;
      rowEdges.push((prev.getBoundingClientRect().bottom + next.getBoundingClientRect().top) / 2 - gridRect.top);
    }
  }
  return { rowEdges, colEdges };
}

function nearestEdge(edges: readonly number[], value: number, threshold = 3): { at: number; dist: number } | null {
  let best: { at: number; dist: number } | null = null;
  for (let at = 0; at < edges.length; at++) {
    const dist = Math.abs(value - edges[at]);
    if (dist <= threshold && (!best || dist < best.dist)) best = { at, dist };
  }
  return best;
}

function trackIndexAt(edges: readonly number[], value: number): number {
  if (edges.length <= 1) return 0;
  for (let i = 0; i < edges.length - 1; i++) {
    if (value >= edges[i] && value <= edges[i + 1]) return i;
  }
  return value < edges[0] ? 0 : edges.length - 2;
}

type RulingHit =
  | { kind: "col"; at: number; row: number }
  | { kind: "row"; at: number; col: number };

function hitRuling(grid: HTMLElement, matrix: { rows: number; cols: number; rowStart?: number; colStart?: number }, clientX: number, clientY: number): RulingHit | null {
  const rowStart = matrix.rowStart ?? 0;
  const colStart = matrix.colStart ?? 0;
  const edges = trackEdges(grid, matrix.rows, matrix.cols, rowStart, colStart);
  if (!edges) return null;
  const rect = grid.getBoundingClientRect();
  const x = clientX - rect.left;
  const y = clientY - rect.top;
  const col = nearestEdge(edges.colEdges, x);
  const row = nearestEdge(edges.rowEdges, y);
  if (col && (!row || col.dist <= row.dist)) {
    return { kind: "col", at: colStart + col.at, row: rowStart + trackIndexAt(edges.rowEdges, y) };
  }
  if (row) return { kind: "row", at: rowStart + row.at, col: colStart + trackIndexAt(edges.colEdges, x) };
  return null;
}

export function SheetGrid(props: { id: string }): JSX.Element {
  return <SheetGridInner id={props.id} depth={0} />;
}

function SheetGridInner(props: { id: string; depth: number }): JSX.Element {
  const surfaceId = useContext(SurfaceContext);
  let gridRef: HTMLDivElement | undefined;
  const [seamStyle, setSeamStyle] = createSignal<JSX.CSSProperties | null>(null);
  const [resizing, setResizing] = createSignal(false);
  const [hovering, setHovering] = createSignal(false);
  const [stableColumns, setStableColumns] = createSignal<string | null>(null);
  const containerOverlay = useContext(SheetContainerOverlayContext);
  const sheetOverlay = props.depth === 0 ? containerOverlay : null;
  const sheetHovering = () => (sheetOverlay?.hovering() ?? false) || hovering();
  const config = createMemo(() => configForBlock(props.id));
  const rowIds = createMemo(() => blockChildren(props.id));
  const hasAggregates = createMemo(() => config().colAggregates.size > 0);
  const footerPinned = createMemo(() => aggregateFooterPinned(props.id));
  const showFooter = createMemo(() => hasAggregates() || footerPinned());
  const [renderLimit, setRenderLimit] = createSignal(GRID_RENDER_PAGE);
  const [rowStart, setRowStart] = createSignal(0);
  const [columnLimit, setColumnLimit] = createSignal(GRID_RENDER_PAGE);
  const [columnStart, setColumnStart] = createSignal(0);
  const [discoveredCols, setDiscoveredCols] = createSignal(1);
  const renderedRows = createMemo(() => Math.min(Math.max(0, rowIds().length - rowStart()), renderLimit()));
  const windowRows = createMemo(() => rowIds()
    .slice(rowStart(), rowStart() + renderedRows())
    .map((id) => ({ id, cellIds: blockChildren(id) })));
  const fullColumnCount = () => discoveredCols();
  let stopDimensionObservation: (() => void) | null = null;
  createEffect(() => {
    const ids = rowIds();
    ids.length;
    stopDimensionObservation?.();
    stopDimensionObservation = observeMatrixDimensions(
      props.id,
      ids,
      (rowId) => blockChildren(rowId).length,
      setDiscoveredCols,
    );
  });
  onCleanup(() => stopDimensionObservation?.());
  createEffect(() => {
    let cols = 1;
    for (const row of windowRows()) cols = Math.max(cols, row.cellIds.length);
    noteMatrixWindowColumns(props.id, cols);
  });
  const renderedCols = createMemo(() => Math.min(
    Math.max(0, fullColumnCount() - columnStart()),
    columnLimit(),
    Math.max(1, Math.floor(GRID_RENDER_CELL_LIMIT / Math.max(1, renderedRows()))),
  ));
  const matrix = createMemo(() => buildMatrixWindow(windowRows(), {
    totalRows: rowIds().length,
    totalCols: fullColumnCount(),
    rowStart: rowStart(),
    colStart: columnStart(),
    visibleCols: renderedCols(),
  }));
  const columns = createMemo(() => columnTracks(renderedCols(), config().colWidths, undefined, columnStart()));
  const editingInThisGrid = () => editingOwner()?.startsWith(`sheet:${surfaceId}:${props.id}:`) ?? false;
  const effectiveColumns = () => stableColumns() ?? columns();
  const showFooterToggle = createMemo(() => !hasAggregates() && (sheetHovering() || footerPinned()));
  const readOnly = () => blockPageReadOnly(props.id);

  const ensureSelectionVisible = (sel: SheetSel) => {
    if (!sel || sel.gridId !== props.id || (sel.surfaceId && sel.surfaceId !== surfaceId)) return;
    const wanted = sel.kind === "row-seam"
      ? Math.max(1, sel.at)
      : sel.kind === "range"
        ? Math.max(sel.anchor.row, sel.focus.row) + 1
        : sel.row + 1;
    if ((wanted <= rowStart() || wanted > rowStart() + renderedRows()) && wanted <= rowIds().length) {
      const next = Math.ceil(wanted / GRID_RENDER_PAGE) * GRID_RENDER_PAGE;
      if (next * renderedCols() <= GRID_RENDER_CELL_LIMIT) {
        setRowStart(0);
        setRenderLimit(next);
      } else {
        const capacity = Math.max(1, Math.floor(GRID_RENDER_CELL_LIMIT / Math.max(1, renderedCols())));
        setRowStart(Math.floor((wanted - 1) / capacity) * capacity);
        setRenderLimit(capacity);
      }
    }
    const wantedCol = sel.kind === "col-seam"
      ? Math.max(1, sel.at)
      : sel.kind === "range"
        ? Math.max(sel.anchor.col, sel.focus.col) + 1
        : sel.col + 1;
    if ((wantedCol <= columnStart() || wantedCol > columnStart() + renderedCols()) && wantedCol <= fullColumnCount()) {
      const next = Math.ceil(wantedCol / GRID_RENDER_PAGE) * GRID_RENDER_PAGE;
      if (next * renderedRows() <= GRID_RENDER_CELL_LIMIT) {
        setColumnStart(0);
        setColumnLimit(next);
      } else {
        const capacity = Math.max(1, Math.floor(GRID_RENDER_CELL_LIMIT / Math.max(1, renderedRows())));
        setColumnStart(Math.floor((wantedCol - 1) / capacity) * capacity);
        setColumnLimit(capacity);
      }
    }
  };

  createEffect(() => {
    const sel = cellSel();
    if (sel) ensureSelectionVisible(sel);
  });

  onCleanup(registerSheetVisibilityHook(props.id, ensureSelectionVisible, surfaceId));
  onCleanup(registerSheetViewAdapter(props.id, {
    bounds: () => ({ rows: rowIds().length, cols: rowIds().length ? fullColumnCount() : 0 }),
    blockIdAt: (row, col) => blockChildren(rowIds()[row] ?? "")[col] ?? null,
  }, surfaceId));

  const toggleFooter = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    toggleAggregateFooterPinned(props.id);
  };

  const footerToggle = () => (
    <SheetAggregateCornerToggle
      active={footerPinned()}
      onClick={toggleFooter}
    />
  );

  createEffect(() => {
    if (hasAggregates() && footerPinned()) setAggregateFooterPinned(props.id, false);
  });

  createEffect(() => {
    if (!sheetOverlay) return;
    sheetOverlay.setCorner(showFooterToggle() ? footerToggle() : null);
  });

  onCleanup(() => sheetOverlay?.setCorner(null));

  const captureStableColumns = () => {
    if (!gridRef) return;
    const tracks = measuredColumnTracks(gridRef, renderedCols(), columnStart());
    if (tracks) setStableColumns(tracks);
  };

  let wasEditing = false;
  createEffect(() => {
    const sel = cellSel();
    columns();
    const editing = editingInThisGrid();
    if (wasEditing && !editing) {
      wasEditing = false;
      setStableColumns(null);
      return;
    }
    wasEditing = editing;
    if (!gridRef || editing) return;
    if (sel && sel.gridId === props.id && sel.kind !== "row-seam" && sel.kind !== "col-seam") captureStableColumns();
    else setStableColumns(null);
  });

  createEffect(() => {
    const sel = cellSel();
    matrix();
    columns();
    if (!gridRef || !sel || sel.gridId !== props.id || (sel.kind !== "row-seam" && sel.kind !== "col-seam")) {
      setSeamStyle(null);
      return;
    }
    const update = () => {
      if (!gridRef) return;
      setSeamStyle(seamStyleFor(gridRef, sel, { rows: renderedRows(), cols: renderedCols(), rowStart: rowStart(), colStart: columnStart() }));
    };
    update();
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(update);
  });

  const previewColumn = (col: number, px: number) => {
    if (!gridRef) return;
    gridRef.style.gridTemplateColumns = columnTracks(renderedCols(), config().colWidths, { col, px }, columnStart());
  };

  const restoreColumns = () => {
    if (gridRef) gridRef.style.gridTemplateColumns = effectiveColumns();
  };

  const onPointerDown = (e: PointerEvent) => {
    if (sheetGridIdFromEventTarget(e.target) !== props.id || isSheetPointerInteractive(e.target)) return;
    // A click inside a nested outline (a cell shown in outline mode, or a cell's
    // rendered children) belongs to that block's own click-to-edit handler. The
    // parent grid must not begin a cell selection here — doing so preventDefaults
    // the pointerdown and the nested block can never be clicked into edit.
    if (e.target instanceof Element && e.target.closest(".sheet-nested-lines")) return;
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey) return;
    const grid = e.currentTarget as HTMLDivElement;
    const hit = e.shiftKey ? null : hitRuling(
      grid,
      { rows: renderedRows(), cols: renderedCols(), rowStart: rowStart(), colStart: columnStart() },
      e.clientX,
      e.clientY,
    );
    if (!hit) {
      beginCellPointerSelection(e, props.id);
      return;
    }

    e.preventDefault();
    e.stopPropagation();

    if (hit.kind === "row") {
      setCellSel(rowSeamSel(props.id, hit.at, hit.col, surfaceId));
      return;
    }

    if (hit.at <= 0) {
      setCellSel(colSeamSel(props.id, hit.at, hit.row, surfaceId));
      return;
    }

    const resizeCol = hit.at - 1;
    const anchor = cellInGrid(grid, 0, resizeCol);
    const startWidth = anchor?.getBoundingClientRect().width ?? 40;
    const startX = e.clientX;
    let dragging = false;
    let lastWidth = Math.max(40, Math.round(startWidth));

    const onMove = (me: PointerEvent) => {
      const dx = me.clientX - startX;
      if (!dragging && Math.abs(dx) < 3) return;
      dragging = true;
      setResizing(true);
      lastWidth = Math.max(40, Math.round(startWidth + dx));
      previewColumn(resizeCol, lastWidth);
      me.preventDefault();
    };
    const finish = (me: PointerEvent) => {
      window.removeEventListener("pointermove", onMove, true);
      window.removeEventListener("pointerup", finish, true);
      window.removeEventListener("pointercancel", cancel, true);
      if (dragging) {
        setColumnWidth(props.id, resizeCol, lastWidth);
        setResizing(false);
      } else {
        setCellSel(colSeamSel(props.id, hit.at, hit.row, surfaceId));
      }
      me.preventDefault();
    };
    const cancel = () => {
      window.removeEventListener("pointermove", onMove, true);
      window.removeEventListener("pointerup", finish, true);
      window.removeEventListener("pointercancel", cancel, true);
      setResizing(false);
      restoreColumns();
    };
    window.addEventListener("pointermove", onMove, true);
    window.addEventListener("pointerup", finish, true);
    window.addEventListener("pointercancel", cancel, true);
  };

  const onDoubleClick = (e: MouseEvent) => {
    const grid = e.currentTarget as HTMLDivElement;
    const hit = hitRuling(grid, { rows: renderedRows(), cols: renderedCols(), rowStart: rowStart(), colStart: columnStart() }, e.clientX, e.clientY);
    if (hit?.kind !== "col" || hit.at <= 0) return;
    e.preventDefault();
    e.stopPropagation();
    setColumnWidth(props.id, hit.at - 1, null);
  };

  const openSheetMenu = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    openSheetContextMenu(e.clientX, e.clientY, props.id, "grid", "children");
  };

  const stopSheetMouseDown = (e: MouseEvent) => {
    if (e.button === 0) e.stopPropagation();
  };

  const stopGrowPointer = (e: MouseEvent | PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
  };

  const growAndEdit = (edge: "row" | "col") => {
    if (readOnly()) return;
    const target = growSheetEdge(props.id, edge, surfaceId);
    if (target) startCellEditing(target);
  };

  const activateEmptyGrid = (e: MouseEvent | KeyboardEvent) => {
    if (readOnly()) return;
    if (e instanceof KeyboardEvent && e.key !== "Enter" && e.key !== " ") return;
    e.preventDefault();
    e.stopPropagation();
    growAndEdit("row");
  };

  const aggregateColumns = createMemo(() => {
    const configured = Array.from({ length: renderedCols() }, (_, index) => columnStart() + index)
      .filter((col) => config().colAggregates.has(`${col}`));
    const aggregateRows = function* () {
      const ids = rowIds();
      for (let row = config().header ? 1 : 0; row < ids.length; row++) {
        yield { cellIds: blockChildren(ids[row]) };
      }
    };
    return collectAggregateColumns(
      aggregateRows(),
      configured,
      (id) => (id ? visibleBody(doc.byId[id]?.raw ?? "").join(" ") : ""),
    );
  });

  return (
    <Show when={props.depth < MAX_GRID_DEPTH} fallback={<SheetOutline ids={blockChildren(props.id)} depth={props.depth} />}>
      <Show
        when={rowIds().length > 0}
        fallback={
          <div
            class="sheet-grid"
            data-sheet-grid-id={props.id}
            data-sheet-surface-id={surfaceId}
            style={{ "grid-template-columns": "max-content" }}
          >
            <div
              class="sheet-cell sheet-grid-placeholder"
              tabIndex={readOnly() ? undefined : 0}
              role={readOnly() ? undefined : "button"}
              onPointerDown={(e) => e.stopPropagation()}
              onMouseDown={(e) => e.stopPropagation()}
              onClick={activateEmptyGrid}
              onKeyDown={activateEmptyGrid}
            />
          </div>
        }
      >
        <div
          ref={(el) => {
            gridRef = el;
          }}
          class="sheet-grid"
          classList={{ "sheet-grid-resizing": resizing() }}
          data-sheet-grid-id={props.id}
          data-sheet-surface-id={surfaceId}
          tabIndex={-1}
          style={{ "grid-template-columns": effectiveColumns() }}
          onPointerDown={onPointerDown}
          onMouseDown={stopSheetMouseDown}
          onPointerEnter={() => setHovering(true)}
          onPointerLeave={() => setHovering(false)}
          onDblClick={onDoubleClick}
          onContextMenu={openSheetMenu}
        >
          <For each={matrix().cells}>
            {(cell) => (
              <SheetGridCell
                gridId={props.id}
                surfaceId={surfaceId}
                cell={cell}
                header={config().header && cell.row === 0}
                depth={props.depth}
                freezeColumns={captureStableColumns}
              />
            )}
          </For>
          <Show when={rowStart() > 0}>
            <button
              type="button"
              class="sheet-add-row-ghost sheet-load-more sheet-load-more-rows"
              style={{ "grid-column": "1 / -1" }}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation();
                const capacity = Math.max(1, Math.floor(GRID_RENDER_CELL_LIMIT / Math.max(1, renderedCols())));
                setRowStart(Math.max(0, rowStart() - capacity));
              }}
            >
              Show previous rows
            </button>
          </Show>
          <Show when={rowStart() + renderedRows() < rowIds().length}>
            <button
              type="button"
              class="sheet-add-row-ghost sheet-load-more"
              style={{ "grid-column": "1 / -1" }}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation();
                const nextLimit = Math.min(
                  renderLimit() + GRID_RENDER_PAGE,
                  Math.floor(GRID_RENDER_CELL_LIMIT / renderedCols()),
                );
                if (nextLimit > renderedRows()) setRenderLimit(nextLimit);
                else setRowStart(rowStart() + renderedRows());
              }}
            >
              {renderedRows() * renderedCols() < GRID_RENDER_CELL_LIMIT ? "Load more rows" : "Show next rows"}
              ({rowStart() + 1}-{rowStart() + renderedRows()} of {rowIds().length})
            </button>
          </Show>
          <Show when={columnStart() > 0}>
            <button
              type="button"
              class="sheet-add-row-ghost sheet-load-more sheet-load-more-columns"
              style={{ "grid-column": "1 / -1" }}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation();
                const capacity = Math.max(1, Math.floor(GRID_RENDER_CELL_LIMIT / Math.max(1, renderedRows())));
                setColumnStart(Math.max(0, columnStart() - capacity));
              }}
            >
              Show previous columns
            </button>
          </Show>
          <Show when={columnStart() + renderedCols() < fullColumnCount()}>
            <button
              type="button"
              class="sheet-add-row-ghost sheet-load-more sheet-load-more-columns"
              style={{ "grid-column": "1 / -1" }}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={(e) => {
                e.stopPropagation();
                const nextLimit = Math.min(
                  columnLimit() + GRID_RENDER_PAGE,
                  Math.floor(GRID_RENDER_CELL_LIMIT / Math.max(1, renderedRows())),
                );
                if (nextLimit > renderedCols()) setColumnLimit(nextLimit);
                else setColumnStart(columnStart() + renderedCols());
              }}
            >
              {renderedCols() * renderedRows() < GRID_RENDER_CELL_LIMIT ? "Load more columns" : "Show next columns"}
              ({columnStart() + 1}-{columnStart() + renderedCols()} of {fullColumnCount()})
            </button>
          </Show>
          <Show when={showFooter()}>
            <For each={Array.from({ length: renderedCols() }, (_, index) => columnStart() + index)}>
              {(col) => (
                <SheetAggregateFooterCell
                  ownerId={props.id}
                  columnKey={`${col}`}
                  fn={config().colAggregates.get(`${col}`) ?? null}
                  values={aggregateColumns().get(col) ?? []}
                  stickyLeft={col === 0}
                  showEmpty={footerPinned()}
                />
              )}
            </For>
          </Show>
          <Show when={!sheetOverlay && showFooterToggle()}>
            {footerToggle()}
          </Show>
          <Show when={props.depth === 0 && !readOnly()}>
            <button
              type="button"
              class="sheet-grid-add-col"
              classList={{ "sheet-grid-add-visible": sheetHovering() }}
              title="Add column"
              onPointerDown={stopGrowPointer}
              onMouseDown={stopGrowPointer}
              onClick={(e) => {
                stopGrowPointer(e);
                growAndEdit("col");
              }}
            >
              +
            </button>
            <button
              type="button"
              class="sheet-grid-add-row"
              classList={{ "sheet-grid-add-visible": sheetHovering() }}
              title="Add row"
              onPointerDown={stopGrowPointer}
              onMouseDown={stopGrowPointer}
              onClick={(e) => {
                stopGrowPointer(e);
                growAndEdit("row");
              }}
            >
              +
            </button>
          </Show>
          <Show when={seamStyle()}>
            {(style) => <div class="sheet-seam-selected" style={style()} />}
          </Show>
        </div>
      </Show>
    </Show>
  );
}

function sameSelectedCell(gridId: string, surfaceId: string, cell: MatrixCell): boolean {
  const sel = cellSel();
  if (!sel || sel.gridId !== gridId || (sel.surfaceId && sel.surfaceId !== surfaceId)) return false;
  if (sel.kind === "cell") return sel.row === cell.row && sel.col === cell.col;
  if (sel.kind === "range") return sel.focus.row === cell.row && sel.focus.col === cell.col;
  return false;
}

function inSelectedRange(gridId: string, surfaceId: string, cell: MatrixCell): boolean {
  const sel = cellSel();
  if (!sel || sel.kind !== "range" || sel.gridId !== gridId || (sel.surfaceId && sel.surfaceId !== surfaceId)) return false;
  const top = Math.min(sel.anchor.row, sel.focus.row);
  const bottom = Math.max(sel.anchor.row, sel.focus.row);
  const left = Math.min(sel.anchor.col, sel.focus.col);
  const right = Math.max(sel.anchor.col, sel.focus.col);
  return cell.row >= top && cell.row <= bottom && cell.col >= left && cell.col <= right;
}

function clickOffset(e: MouseEvent, contentRef: HTMLDivElement | undefined, raw: string): number | null {
  if (!contentRef) return null;
  const d = document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
  const range = d.caretRangeFromPoint?.(e.clientX, e.clientY);
  if (!range) return null;
  return editorOffsetFromRenderedRange(contentRef, range, raw, isSheetCellHidden);
}

function SheetGridCell(props: { gridId: string; surfaceId: string; cell: MatrixCell; header: boolean; depth: number; freezeColumns: () => void }): JSX.Element {
  const sel = (): SheetCellCtx => ({ gridId: props.gridId, surfaceId: props.surfaceId, row: props.cell.row, col: props.cell.col });
  let contentRef: HTMLDivElement | undefined;
  const bgColor = createMemo(() => {
    const id = props.cell.blockId;
    const node = id ? doc.byId[id] : null;
    return node ? blockBackgroundColor(facetsOf(node.raw, formatForBlock(id!)).properties) : undefined;
  });
  const onDoubleClick = (e: MouseEvent) => {
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey) return;
    if (forbidsEditEntry(e)) return;
    e.preventDefault();
    e.stopPropagation();
    props.freezeColumns();
    const blockId = props.cell.blockId;
    if (!blockId) return;
    const node = doc.byId[blockId];
    const offset = node ? clickOffset(e, contentRef, node.raw) : null;
    startCellEditing(sel(), offset ?? undefined);
  };
  const removeCtx = () => ({
    rowId: doc.byId[props.gridId]?.children[props.cell.row],
    gridId: props.gridId,
    col: props.cell.col,
  });
  const openCellMenu = (e: MouseEvent) => {
    const blockId = props.cell.blockId;
    if (!blockId) return;
    e.preventDefault();
    e.stopPropagation();
    setCellSel(sel());
    openSheetCellContextMenu(e.clientX, e.clientY, blockId, removeCtx());
  };
  const openCellMenuFromHandle = (e: MouseEvent) => {
    const blockId = props.cell.blockId;
    if (!blockId) return;
    e.preventDefault();
    e.stopPropagation();
    setCellSel(sel());
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openSheetCellContextMenu(rect.right, rect.bottom + 2, blockId, removeCtx());
  };

  return (
    <div
      class="sheet-cell"
      classList={{
        "sheet-header-cell": props.header,
        "sheet-sticky-left": props.cell.col === 0,
        "sheet-hole": !props.cell.blockId,
        "sheet-cell-in-range": inSelectedRange(props.gridId, props.surfaceId, props.cell),
        "sheet-cell-selected": sameSelectedCell(props.gridId, props.surfaceId, props.cell),
      }}
      data-sheet-grid-id={props.gridId}
      data-sheet-surface-id={props.surfaceId}
      data-block-id={props.cell.blockId ?? undefined}
      data-row={props.cell.row}
      data-col={props.cell.col}
      style={bgColor() ? { background: bgColor() } : undefined}
      onDblClick={onDoubleClick}
      onContextMenu={openCellMenu}
    >
      <Show when={props.cell.blockId}>
        <button
          class="sheet-cell-handle"
          title="Cell menu"
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
      <Show when={props.cell.blockId}>
        {(blockId) => (
          <SheetBlock
            id={blockId()}
            depth={props.depth + 1}
            cell={sel()}
            bodyRef={(el) => {
              contentRef = el;
            }}
          />
      )}
      </Show>
    </div>
  );
}

function SheetBlock(props: {
  id: string;
  depth: number;
  cell?: SheetCellCtx;
  nested?: boolean;
  bodyRef?: (el: HTMLDivElement) => void;
}): JSX.Element {
  let contentRef: HTMLDivElement | undefined;
  const node = () => doc.byId[props.id];
  const fmt = () => formatForBlock(props.id);
  const facets = createMemo(() => (node() ? facetsOf(node().raw, fmt()) : null));
  const config = createMemo(() => (facets() ? sheetConfig(facets()!.properties) : null));
  const children = () => node()?.children ?? [];
  const editing = () => {
    const cell = props.cell;
    return !!cell && editingId() === props.id && editingOwner() === cellOwner(cell);
  };
  const bodyRef = (el: HTMLDivElement) => {
    contentRef = el;
    props.bodyRef?.(el);
  };
  const onNestedMouseDown = (e: MouseEvent) => {
    if (!props.nested || !props.cell) return;
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
    if (blockPageReadOnly(props.id)) return;
    if (forbidsEditEntry(e)) return;
    e.preventDefault();
    e.stopPropagation();
    const n = node();
    if (!n) return;
    const fallback = splitProps(n.raw, isSheetCellHidden).visible.length;
    setCellSel(props.cell);
    startEditing(props.id, clickOffset(e, contentRef, n.raw) ?? fallback, cellOwner(props.cell));
  };

  return (
    <Show when={node()}>
      {(n) => (
        <>
          <div
            class="sheet-cell-body"
            ref={bodyRef}
            onMouseDown={onNestedMouseDown}
          >
            <Show
              when={editing() && props.cell}
              fallback={<AstBody raw={n().raw} format={fmt()} headingLevel={facets()?.headingLevel ?? null} />}
            >
              {(cell) => (
                <SheetCellContext.Provider value={cell()}>
                  <SurfaceContext.Provider value={cellSurfaceKey(cell().gridId, cell().surfaceId)}>
                    <Editor id={props.id} />
                  </SurfaceContext.Provider>
                </SheetCellContext.Provider>
              )}
            </Show>
          </div>
          <Show when={children().length > 0 || config()?.view === "grid" || config()?.view === "table" || config()?.view === "board"}>
            <SheetContainerOverlayContext.Provider value={null}>
              <Switch>
                <Match when={config()?.view === "grid"}>
                  <SheetGridInner id={props.id} depth={props.depth} />
                </Match>
                <Match when={config()?.view === "table"}>
                  <SheetTable ownerId={props.id} rowSource="children" />
                </Match>
                <Match when={config()?.view === "board"}>
                  <SheetBoard ownerId={props.id} rowSource="children" groupBy={config()?.groupBy} />
                </Match>
                <Match when={true}>
                  <SheetOutline ids={children()} depth={props.depth} cell={props.cell} />
                </Match>
              </Switch>
            </SheetContainerOverlayContext.Provider>
          </Show>
        </>
      )}
    </Show>
  );
}

function SheetOutline(props: { ids: readonly string[]; depth: number; cell?: SheetCellCtx }): JSX.Element {
  return (
    <div class="sheet-nested-lines">
      <For each={props.ids}>
            {(id) => (
          <div class="sheet-nested-line" style={{ "padding-left": `${Math.max(0, props.depth) * 14}px` }}>
            <SheetBlock id={id} depth={props.depth + 1} cell={props.cell} nested />
          </div>
        )}
      </For>
    </div>
  );
}
