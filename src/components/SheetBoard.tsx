import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { doc, ensurePageLoaded, formatForBlock, formatForPage, pageByName } from "../store";
import { facetsFromDto, facetsOf, type Facets } from "../render/facets";
import { visibleBody } from "../render/block";
import { InlineText } from "../render/inline";
import { editingId, editingOwner } from "../editorController";
import { SheetCellContext, type SheetCellCtx } from "../sheet/context";
import {
  cellOwner,
  cellSel,
  cellSurfaceKey,
  registerSheetViewAdapter,
  setCellSel,
  startCellEditing,
  type CellSel,
} from "../sheet/selection";
import {
  groupKeyForBlock,
  isFieldId,
  readField,
  writeField,
  type FieldId,
} from "../sheet/fields";
import { MARKERS } from "../markers";
import { workflow } from "../ui";
import type { BlockDto, RefGroup } from "../types";
import { Editor, SurfaceContext } from "./Block";

interface RowRecord {
  id: string;
  page: string;
  dto?: BlockDto;
}

interface BoardColumn {
  key: string | null;
  label: string;
  rows: RowRecord[];
}

const NONE_LABEL = "(none)";

export function SheetBoard(props: {
  ownerId: string;
  rowSource: "children" | "query";
  groupBy?: string | null;
  groups?: readonly RefGroup[];
}): JSX.Element {
  const groupBy = createMemo<FieldId>(() => {
    const raw = props.groupBy || "state";
    return isFieldId(raw) ? raw : "state";
  });
  const [drag, setDrag] = createSignal<{ id: string; col: number; row: number; overCol: number | null } | null>(null);

  const queryPages = createMemo(() => {
    const map = new Map<string, RefGroup>();
    for (const group of props.groups ?? []) map.set(`${group.kind}\0${group.page}`, group);
    return [...map.values()];
  });
  const [pagesReady] = createResource(
    () => (props.rowSource === "query" ? queryPages().map((g) => `${g.kind}:${g.page}`).join("\0") : null),
    async () => {
      await Promise.all(
        queryPages().map(async (g) => {
          if (pageByName(g.page)) return;
          const dto = await backend().getPage(g.page, g.kind);
          if (dto) ensurePageLoaded(dto);
        })
      );
      return true;
    }
  );
  void pagesReady;

  const rows = createMemo<RowRecord[]>(() => {
    if (props.rowSource === "children") {
      return (doc.byId[props.ownerId]?.children ?? []).map((id) => ({
        id,
        page: doc.byId[id]?.page ?? doc.byId[props.ownerId]?.page ?? "",
      }));
    }
    return (props.groups ?? []).flatMap((g) => g.blocks.map((b) => ({ id: b.id, page: g.page, dto: b })));
  });

  const columns = createMemo<BoardColumn[]>(() => buildColumns(rows(), groupBy()));
  const maxRows = createMemo(() => Math.max(1, ...columns().map((c) => c.rows.length)));

  const selected = (col: number, row: number) => {
    const sel = cellSel();
    if (!sel || sel.gridId !== props.ownerId) return false;
    if (sel.kind === "cell") return sel.col === col && sel.row === row;
    if (sel.kind === "range") return sel.focus.col === col && sel.focus.row === row;
    return false;
  };

  const moveCard = (sel: CellSel, dir: "left" | "right"): boolean => {
    const cols = columns();
    const from = cols[sel.col];
    const row = from?.rows[sel.row];
    const targetCol = dir === "left" ? sel.col - 1 : sel.col + 1;
    const target = cols[targetCol];
    if (!row || !target || !doc.byId[row.id]) return true;
    if (writeField(row.id, groupBy(), target.key ?? "")) {
      const nextRows = columns()[targetCol]?.rows ?? [];
      setCellSel({ gridId: props.ownerId, col: targetCol, row: Math.max(0, nextRows.findIndex((r) => r.id === row.id)) });
    }
    return true;
  };

  onMount(() => {
    const dispose = registerSheetViewAdapter(props.ownerId, {
      bounds: () => ({ rows: maxRows(), cols: columns().length }),
      blockIdAt: (row, col) => columns()[col]?.rows[row]?.id ?? null,
      activate: (sel) => {
        const row = columns()[sel.col]?.rows[sel.row];
        if (!row || !doc.byId[row.id]) return true;
        return false;
      },
      overtype: () => true,
      moveWithMod: (sel, dir) => {
        if (dir === "left" || dir === "right") return moveCard(sel, dir);
        return true;
      },
    });
    onCleanup(dispose);
  });

  const dropCard = (row: RowRecord, colIndex: number, targetCol: number | null) => {
    if (targetCol == null || targetCol === colIndex || !doc.byId[row.id]) return;
    const target = columns()[targetCol];
    if (target) writeField(row.id, groupBy(), target.key ?? "");
  };

  return (
    <Show when={columns().length > 0} fallback={<div class="sheet-board sheet-empty">empty board</div>}>
      <div class="sheet-board" data-sheet-grid-id={props.ownerId}>
        <For each={columns()}>
          {(col, colIndex) => (
            <section
              class="sheet-board-column"
              classList={{ "sheet-board-drop": drag()?.overCol === colIndex() }}
              data-board-col={colIndex()}
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
                      row={row}
                      groupBy={groupBy()}
                      colIndex={colIndex()}
                      rowIndex={rowIndex()}
                      selected={selected(colIndex(), rowIndex())}
                      dragging={drag()?.id === row.id}
                      setDrag={setDrag}
                      dropCard={dropCard}
                    />
                  )}
                </For>
              </div>
            </section>
          )}
        </For>
      </div>
    </Show>
  );
}

function buildColumns(rows: readonly RowRecord[], groupBy: FieldId): BoardColumn[] {
  const keys = rows.map((row) => groupKey(row, groupBy));
  let order: (string | null)[];
  if (groupBy === "state") {
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
  if (keys.includes(null)) order.push(null);
  if (order.length === 0) order = [null];
  return order.map((key) => ({
    key,
    label: key === null ? NONE_LABEL : groupBy === "priority" ? `[#${key}]` : key,
    rows: rows.filter((row, i) => keys[i] === key),
  }));
}

function groupKey(row: RowRecord, field: FieldId): string | null {
  if (doc.byId[row.id]) return groupKeyForBlock(row.id, field);
  const v = dtoField(row, field);
  return v || null;
}

function recordFacets(row: RowRecord): Facets | null {
  const n = doc.byId[row.id];
  if (n) return facetsOf(n.raw, formatForBlock(row.id));
  return row.dto ? facetsFromDto(row.dto) : null;
}

function dtoField(row: RowRecord, field: FieldId): string | null {
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
  return doc.byId[row.id]?.raw ?? row.dto?.raw ?? "";
}

function rowTitle(row: RowRecord): string {
  return visibleBody(rowRaw(row))[0] ?? "";
}

function BoardCard(props: {
  ownerId: string;
  row: RowRecord;
  groupBy: FieldId;
  colIndex: number;
  rowIndex: number;
  selected: boolean;
  dragging: boolean;
  setDrag: (v: { id: string; col: number; row: number; overCol: number | null } | null) => void;
  dropCard: (row: RowRecord, colIndex: number, targetCol: number | null) => void;
}): JSX.Element {
  const cell = (): SheetCellCtx => ({ gridId: props.ownerId, row: props.rowIndex, col: props.colIndex });
  const editing = () => editingId() === props.row.id && editingOwner() === cellOwner(cell());
  const fmt = () => (doc.byId[props.row.id] ? formatForBlock(props.row.id) : formatForPage(props.row.page));

  const select = () => setCellSel(cell());

  const beginPointerDrag = (e: PointerEvent) => {
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
    const startX = e.clientX;
    const startY = e.clientY;
    let moved = false;
    let overCol: number | null = null;
    const onMove = (ev: PointerEvent) => {
      if (!moved && Math.hypot(ev.clientX - startX, ev.clientY - startY) < 4) return;
      moved = true;
      const el = (document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null)?.closest(".sheet-board-column") as HTMLElement | null;
      const idx = el ? Number(el.dataset.boardCol) : NaN;
      overCol = Number.isFinite(idx) ? idx : null;
      props.setDrag({ id: props.row.id, col: props.colIndex, row: props.rowIndex, overCol });
      ev.preventDefault();
    };
    const onUp = (ev: PointerEvent) => {
      document.removeEventListener("pointermove", onMove, true);
      document.removeEventListener("pointerup", onUp, true);
      document.removeEventListener("pointercancel", onCancel, true);
      if (moved) {
        props.dropCard(props.row, props.colIndex, overCol);
        props.setDrag(null);
        ev.preventDefault();
      }
    };
    const onCancel = () => {
      document.removeEventListener("pointermove", onMove, true);
      document.removeEventListener("pointerup", onUp, true);
      document.removeEventListener("pointercancel", onCancel, true);
      props.setDrag(null);
    };
    document.addEventListener("pointermove", onMove, true);
    document.addEventListener("pointerup", onUp, true);
    document.addEventListener("pointercancel", onCancel, true);
  };

  const onClick = (e: MouseEvent) => {
    e.stopPropagation();
    select();
    if (doc.byId[props.row.id]) startCellEditing(cell());
  };

  return (
    <article
      class="sheet-board-card"
      classList={{
        "sheet-cell-selected": props.selected,
        "sheet-board-card-dragging": props.dragging,
      }}
      data-sheet-grid-id={props.ownerId}
      data-row={props.rowIndex}
      data-col={props.colIndex}
      data-block-id={props.row.id}
      onPointerDown={beginPointerDrag}
      onClick={onClick}
    >
      <Show
        when={editing()}
        fallback={
          <>
            <div class="sheet-board-card-title">
              <InlineText text={rowTitle(props.row)} format={fmt()} />
            </div>
            <CardChips row={props.row} groupBy={props.groupBy} />
          </>
        }
      >
        <SheetCellContext.Provider value={cell()}>
          <SurfaceContext.Provider value={cellSurfaceKey(props.ownerId)}>
            <Editor id={props.row.id} />
          </SurfaceContext.Provider>
        </SheetCellContext.Provider>
      </Show>
    </article>
  );
}

function CardChips(props: { row: RowRecord; groupBy: FieldId }): JSX.Element {
  const value = (field: FieldId) => (doc.byId[props.row.id] ? readField(props.row.id, field)?.text ?? "" : dtoField(props.row, field) ?? "");
  const priority = () => props.groupBy === "priority" ? "" : value("priority");
  const scheduled = () => props.groupBy === "scheduled" ? "" : value("scheduled");
  const deadline = () => props.groupBy === "deadline" ? "" : value("deadline");
  const tags = () => props.groupBy === "tags" ? "" : value("tags");
  return (
    <div class="sheet-board-card-chips">
      <Show when={priority()}>
        <span class="block-priority">{priority()}</span>
      </Show>
      <Show when={scheduled()}>
        <span class="date-chip scheduled">{scheduled()}</span>
      </Show>
      <Show when={deadline()}>
        <span class="date-chip deadline">{deadline()}</span>
      </Show>
      <Show when={tags()}>
        <For each={tags().split(/\s+/).filter(Boolean)}>
          {(tag) => <span class="sheet-tag-chip">{tag.startsWith("#") ? tag : `#${tag}`}</span>}
        </For>
      </Show>
    </div>
  );
}
