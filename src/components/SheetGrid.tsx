import { For, Match, Show, Switch, createEffect, createMemo, createSignal, type JSX } from "solid-js";
import { doc, formatForBlock } from "../store";
import { AstBody } from "../render/body";
import { visibleBody } from "../render/block";
import { facetsOf } from "../render/facets";
import { sheetConfig } from "../sheet/config";
import { buildMatrix, type MatrixCell } from "../sheet/matrix";
import { SheetCellContext, type SheetCellCtx } from "../sheet/context";
import {
  cellOwner,
  cellSel,
  cellSurfaceKey,
  setCellSel,
  startCellEditing,
  type SheetSel,
} from "../sheet/selection";
import { setColumnWidth } from "../sheet/mutations";
import { editorOffsetFromRenderedRange } from "../render/spans";
import { isSheetCellHidden } from "../editor/properties";
import { forbidsEditEntry } from "../editor/editTargets";
import { editingId, editingOwner } from "../editorController";
import { openSheetCellContextMenu, openSheetContextMenu } from "../ui";
import { blockBackgroundColor } from "../blockColors";
import { Editor, SurfaceContext } from "./Block";
import { SheetTable } from "./SheetTable";
import { SheetBoard } from "./SheetBoard";
import { SheetAggregateFooterCell } from "./SheetAggregateFooter";

const MAX_GRID_DEPTH = 5;

function configForBlock(id: string) {
  const node = doc.byId[id];
  return sheetConfig(node ? facetsOf(node.raw, formatForBlock(id)).properties : []);
}

function blockChildren(id: string): string[] {
  return doc.byId[id]?.children ?? [];
}

function columnTracks(cols: number, widths: ReadonlyMap<number, number>, preview?: { col: number; px: number }): string {
  const tracks: string[] = [];
  for (let col = 0; col < cols; col++) {
    const px = preview?.col === col ? preview.px : widths.get(col);
    tracks.push(px == null ? "max-content" : `${px}px`);
  }
  return tracks.join(" ");
}

function cellInGrid(grid: HTMLElement, row: number, col: number): HTMLElement | null {
  return grid.querySelector(`:scope > .sheet-cell[data-row="${row}"][data-col="${col}"]`) as HTMLElement | null;
}

function seamStyleFor(grid: HTMLElement, sel: SheetSel, matrix: { rows: number; cols: number }): JSX.CSSProperties | null {
  if ((sel.kind !== "row-seam" && sel.kind !== "col-seam") || matrix.rows <= 0 || matrix.cols <= 0) return null;
  const gridRect = grid.getBoundingClientRect();

  if (sel.kind === "row-seam") {
    const row = Math.max(0, Math.min(sel.at, matrix.rows - 1));
    const cell = cellInGrid(grid, row, 0);
    if (!cell) return null;
    const rect = cell.getBoundingClientRect();
    const y = sel.at <= 0 ? rect.top - gridRect.top : sel.at >= matrix.rows ? rect.bottom - gridRect.top : rect.top - gridRect.top;
    // Clamp into the visible content box: the outermost boundary seams would
    // otherwise land exactly on the overflow clip edge and paint nothing.
    const top = Math.max(0, Math.min(Math.round(y + grid.scrollTop - 1), grid.scrollHeight - 2));
    return {
      left: "0px",
      top: `${top}px`,
      width: `${grid.scrollWidth}px`,
      height: "2px",
    };
  }

  const col = Math.max(0, Math.min(sel.at, matrix.cols - 1));
  const cell = cellInGrid(grid, 0, col);
  if (!cell) return null;
  const rect = cell.getBoundingClientRect();
  const x = sel.at <= 0 ? rect.left - gridRect.left : sel.at >= matrix.cols ? rect.right - gridRect.left : rect.left - gridRect.left;
  // Same boundary clamp as row seams (right-edge seam vs the overflow clip).
  const left = Math.max(0, Math.min(Math.round(x + grid.scrollLeft - 1), grid.scrollWidth - 2));
  return {
    left: `${left}px`,
    top: "0px",
    width: "2px",
    height: `${grid.scrollHeight}px`,
  };
}

function trackEdges(grid: HTMLElement, rows: number, cols: number): { rowEdges: number[]; colEdges: number[] } | null {
  if (rows <= 0 || cols <= 0) return null;
  const gridRect = grid.getBoundingClientRect();
  const colEdges: number[] = [];
  for (let at = 0; at <= cols; at++) {
    if (at === 0) {
      const first = cellInGrid(grid, 0, 0);
      if (!first) return null;
      colEdges.push(first.getBoundingClientRect().left - gridRect.left);
    } else if (at === cols) {
      const last = cellInGrid(grid, 0, cols - 1);
      if (!last) return null;
      colEdges.push(last.getBoundingClientRect().right - gridRect.left);
    } else {
      const prev = cellInGrid(grid, 0, at - 1);
      const next = cellInGrid(grid, 0, at);
      if (!prev || !next) return null;
      colEdges.push((prev.getBoundingClientRect().right + next.getBoundingClientRect().left) / 2 - gridRect.left);
    }
  }

  const rowEdges: number[] = [];
  for (let at = 0; at <= rows; at++) {
    if (at === 0) {
      const first = cellInGrid(grid, 0, 0);
      if (!first) return null;
      rowEdges.push(first.getBoundingClientRect().top - gridRect.top);
    } else if (at === rows) {
      const last = cellInGrid(grid, rows - 1, 0);
      if (!last) return null;
      rowEdges.push(last.getBoundingClientRect().bottom - gridRect.top);
    } else {
      const prev = cellInGrid(grid, at - 1, 0);
      const next = cellInGrid(grid, at, 0);
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

function hitRuling(grid: HTMLElement, matrix: { rows: number; cols: number }, clientX: number, clientY: number): RulingHit | null {
  const edges = trackEdges(grid, matrix.rows, matrix.cols);
  if (!edges) return null;
  const rect = grid.getBoundingClientRect();
  const x = clientX - rect.left;
  const y = clientY - rect.top;
  const col = nearestEdge(edges.colEdges, x);
  const row = nearestEdge(edges.rowEdges, y);
  if (col && (!row || col.dist <= row.dist)) {
    return { kind: "col", at: col.at, row: trackIndexAt(edges.rowEdges, y) };
  }
  if (row) return { kind: "row", at: row.at, col: trackIndexAt(edges.colEdges, x) };
  return null;
}

export function SheetGrid(props: { id: string }): JSX.Element {
  return <SheetGridInner id={props.id} depth={0} />;
}

function SheetGridInner(props: { id: string; depth: number }): JSX.Element {
  let gridRef: HTMLDivElement | undefined;
  const [seamStyle, setSeamStyle] = createSignal<JSX.CSSProperties | null>(null);
  const [resizing, setResizing] = createSignal(false);
  const [hovering, setHovering] = createSignal(false);
  const config = createMemo(() => configForBlock(props.id));
  const rows = createMemo(() =>
    blockChildren(props.id).map((id) => ({
      id,
      cellIds: blockChildren(id),
    }))
  );
  const matrix = createMemo(() => buildMatrix(rows()));
  const columns = createMemo(() => columnTracks(matrix().cols, config().colWidths));
  const hasAggregates = createMemo(() => config().colAggregates.size > 0);

  createEffect(() => {
    const sel = cellSel();
    const m = matrix();
    columns();
    if (!gridRef || !sel || sel.gridId !== props.id || (sel.kind !== "row-seam" && sel.kind !== "col-seam")) {
      setSeamStyle(null);
      return;
    }
    const update = () => {
      if (!gridRef) return;
      setSeamStyle(seamStyleFor(gridRef, sel, m));
    };
    update();
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(update);
  });

  const previewColumn = (col: number, px: number) => {
    if (!gridRef) return;
    gridRef.style.gridTemplateColumns = columnTracks(matrix().cols, config().colWidths, { col, px });
  };

  const restoreColumns = () => {
    if (gridRef) gridRef.style.gridTemplateColumns = columns();
  };

  const onPointerDown = (e: PointerEvent) => {
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
    const grid = e.currentTarget as HTMLDivElement;
    const hit = hitRuling(grid, matrix(), e.clientX, e.clientY);
    if (!hit) return;
    e.preventDefault();
    e.stopPropagation();

    if (hit.kind === "row") {
      setCellSel({ kind: "row-seam", gridId: props.id, at: hit.at, col: hit.col });
      return;
    }

    if (hit.at <= 0) {
      setCellSel({ kind: "col-seam", gridId: props.id, at: hit.at, row: hit.row });
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
        setCellSel({ kind: "col-seam", gridId: props.id, at: hit.at, row: hit.row });
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
    const hit = hitRuling(grid, matrix(), e.clientX, e.clientY);
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

  const columnValues = (col: number): string[] =>
    matrix().cells
      .filter((cell) => cell.col === col && !(config().header && cell.row === 0))
      .map((cell) => (cell.blockId ? visibleBody(doc.byId[cell.blockId]?.raw ?? "").join(" ") : ""));

  return (
    <Show when={props.depth < MAX_GRID_DEPTH} fallback={<SheetOutline ids={blockChildren(props.id)} depth={props.depth} />}>
      <Show when={rows().length > 0} fallback={<div class="sheet-grid sheet-empty">empty grid</div>}>
        <div
          ref={(el) => {
            gridRef = el;
          }}
          class="sheet-grid"
          classList={{ "sheet-grid-resizing": resizing() }}
          data-sheet-grid-id={props.id}
          tabIndex={-1}
          style={{ "grid-template-columns": columns() }}
          onPointerDown={onPointerDown}
          onPointerEnter={() => setHovering(true)}
          onPointerLeave={() => setHovering(false)}
          onDblClick={onDoubleClick}
          onContextMenu={openSheetMenu}
        >
          <For each={matrix().cells}>
            {(cell) => (
              <SheetGridCell
                gridId={props.id}
                cell={cell}
                header={config().header && cell.row === 0}
                depth={props.depth}
              />
            )}
          </For>
          <Show when={hasAggregates() || hovering()}>
            <For each={Array.from({ length: matrix().cols }, (_, col) => col)}>
              {(col) => (
                <SheetAggregateFooterCell
                  ownerId={props.id}
                  columnKey={`${col}`}
                  fn={config().colAggregates.get(`${col}`) ?? null}
                  values={columnValues(col)}
                  stickyLeft={col === 0}
                  showEmpty={hovering()}
                />
              )}
            </For>
          </Show>
          <Show when={seamStyle()}>
            {(style) => <div class="sheet-seam-selected" style={style()} />}
          </Show>
        </div>
      </Show>
    </Show>
  );
}

function sameSelectedCell(gridId: string, cell: MatrixCell): boolean {
  const sel = cellSel();
  if (!sel || sel.gridId !== gridId) return false;
  if (sel.kind === "cell") return sel.row === cell.row && sel.col === cell.col;
  if (sel.kind === "range") return sel.focus.row === cell.row && sel.focus.col === cell.col;
  return false;
}

function inSelectedRange(gridId: string, cell: MatrixCell): boolean {
  const sel = cellSel();
  if (!sel || sel.kind !== "range" || sel.gridId !== gridId) return false;
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

function SheetGridCell(props: { gridId: string; cell: MatrixCell; header: boolean; depth: number }): JSX.Element {
  const sel = (): SheetCellCtx => ({ gridId: props.gridId, row: props.cell.row, col: props.cell.col });
  let contentRef: HTMLDivElement | undefined;
  const bgColor = createMemo(() => {
    const id = props.cell.blockId;
    const node = id ? doc.byId[id] : null;
    return node ? blockBackgroundColor(facetsOf(node.raw, formatForBlock(id!)).properties) : undefined;
  });
  const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
    if (forbidsEditEntry(e)) return;
    e.preventDefault();
    e.stopPropagation();
    const blockId = props.cell.blockId;
    if (!blockId) {
      setCellSel(sel());
      return;
    }
    const node = doc.byId[blockId];
    const offset = node ? clickOffset(e, contentRef, node.raw) : null;
    if (offset == null) setCellSel(sel());
    else startCellEditing(sel(), offset);
  };
  const openCellMenu = (e: MouseEvent) => {
    const blockId = props.cell.blockId;
    if (!blockId) return;
    e.preventDefault();
    e.stopPropagation();
    setCellSel(sel());
    openSheetCellContextMenu(e.clientX, e.clientY, blockId);
  };
  const openCellMenuFromHandle = (e: MouseEvent) => {
    const blockId = props.cell.blockId;
    if (!blockId) return;
    e.preventDefault();
    e.stopPropagation();
    setCellSel(sel());
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openSheetCellContextMenu(rect.right, rect.bottom + 2, blockId);
  };

  return (
    <div
      class="sheet-cell"
      classList={{
        "sheet-header-cell": props.header,
        "sheet-sticky-left": props.cell.col === 0,
        "sheet-hole": !props.cell.blockId,
        "sheet-cell-in-range": inSelectedRange(props.gridId, props.cell),
        "sheet-cell-selected": sameSelectedCell(props.gridId, props.cell),
      }}
      data-sheet-grid-id={props.gridId}
      data-block-id={props.cell.blockId ?? undefined}
      data-row={props.cell.row}
      data-col={props.cell.col}
      style={bgColor() ? { background: bgColor() } : undefined}
      onMouseDown={onMouseDown}
      onContextMenu={openCellMenu}
    >
      <Show when={props.cell.blockId}>
        <button
          class="sheet-cell-handle"
          title="Cell menu"
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

function SheetBlock(props: { id: string; depth: number; cell?: SheetCellCtx; bodyRef?: (el: HTMLDivElement) => void }): JSX.Element {
  const node = () => doc.byId[props.id];
  const fmt = () => formatForBlock(props.id);
  const facets = createMemo(() => (node() ? facetsOf(node().raw, fmt()) : null));
  const config = createMemo(() => (facets() ? sheetConfig(facets()!.properties) : null));
  const children = () => node()?.children ?? [];
  const editing = () => {
    const cell = props.cell;
    return !!cell && editingId() === props.id && editingOwner() === cellOwner(cell);
  };

  return (
    <Show when={node()}>
      {(n) => (
        <>
          <div
            class="sheet-cell-body"
            ref={(el) => props.bodyRef?.(el)}
          >
            <Show
              when={editing() && props.cell}
              fallback={<AstBody raw={n().raw} format={fmt()} headingLevel={facets()?.headingLevel ?? null} />}
            >
              {(cell) => (
                <SheetCellContext.Provider value={cell()}>
                  <SurfaceContext.Provider value={cellSurfaceKey(cell().gridId)}>
                    <Editor id={props.id} />
                  </SurfaceContext.Provider>
                </SheetCellContext.Provider>
              )}
            </Show>
          </div>
          <Show when={children().length > 0 || config()?.view === "grid" || config()?.view === "table" || config()?.view === "board"}>
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
                <SheetOutline ids={children()} depth={props.depth} />
              </Match>
            </Switch>
          </Show>
        </>
      )}
    </Show>
  );
}

function SheetOutline(props: { ids: readonly string[]; depth: number }): JSX.Element {
  return (
    <div class="sheet-nested-lines">
      <For each={props.ids}>
            {(id) => (
          <div class="sheet-nested-line" style={{ "padding-left": `${Math.max(0, props.depth) * 14}px` }}>
            <SheetBlock id={id} depth={props.depth + 1} />
          </div>
        )}
      </For>
    </div>
  );
}
