import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { doc, ensurePageLoaded, formatForBlock, formatForPage, pageByName, readPageProperty } from "../store";
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
  groupKeysForBlock,
  isFieldId,
  readField,
  writeTagDelta,
  writeField,
  type FieldId,
} from "../sheet/fields";
import { parseFields, sheetConfig, type FieldSpec } from "../sheet/config";
import { MARKERS } from "../markers";
import { openSheetCellContextMenu, openSheetContextMenu, workflow } from "../ui";
import { blockBackgroundColor } from "../blockColors";
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
  schemaPage?: string;
}): JSX.Element {
  const groupBy = createMemo<FieldId>(() => {
    const raw = props.groupBy || "state";
    return isFieldId(raw) ? raw : "state";
  });
  const [drag, setDrag] = createSignal<{ id: string; col: number; row: number; overCol: number | null } | null>(null);
  const config = createMemo(() => {
    const owner = doc.byId[props.ownerId];
    return sheetConfig(owner ? facetsOf(owner.raw, formatForBlock(props.ownerId)).properties : []);
  });
  const schemaFields = createMemo<readonly FieldSpec[]>(() => {
    const own = config().fields;
    if (own.length > 0) return own;
    return props.schemaPage ? parseFields(readPageProperty(props.schemaPage, "tine.fields") ?? "") : [];
  });

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

  const columns = createMemo<BoardColumn[]>(() => buildColumns(rows(), groupBy(), schemaFields()));
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
    if (moveRowToColumn(row, from.key, target.key, groupBy())) {
      const nextCols = columns();
      const nextCol = Math.max(0, nextCols.findIndex((c) => c.key === target.key));
      const nextRows = nextCols[nextCol]?.rows ?? [];
      setCellSel({ gridId: props.ownerId, col: nextCol, row: Math.max(0, nextRows.findIndex((r) => r.id === row.id)) });
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
    const cols = columns();
    const from = cols[colIndex];
    const target = cols[targetCol];
    if (from && target) moveRowToColumn(row, from.key, target.key, groupBy());
  };

  const openSheetMenu = (e: MouseEvent) => {
    if (props.rowSource !== "children") return;
    e.preventDefault();
    e.stopPropagation();
    openSheetContextMenu(e.clientX, e.clientY, props.ownerId, "board", props.rowSource, groupBy());
  };

  return (
    <Show when={columns().length > 0} fallback={<div class="sheet-board sheet-empty">empty board</div>}>
      <div class="sheet-board" data-sheet-grid-id={props.ownerId} onContextMenu={openSheetMenu}>
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
                      dragging={drag()?.id === row.id && drag()?.col === colIndex() && drag()?.row === rowIndex()}
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

function buildColumns(rows: readonly RowRecord[], groupBy: FieldId, schema: readonly FieldSpec[] = []): BoardColumn[] {
  const keySets = rows.map((row) => groupKeysForBlock(row, groupBy));
  const keys = keySets.map((ks) => ks[0] ?? null);
  let order: (string | null)[];
  const enumValues = enumValuesFor(schema, groupBy);
  if (groupBy === "tags") {
    order = [];
    for (const ks of keySets) {
      for (const key of ks) if (key !== null && !order.includes(key)) order.push(key);
    }
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
  if (keySets.some((ks) => ks.includes(null)) && !order.includes(null)) order.push(null);
  if (order.length === 0) order = [null];
  return order.map((key) => ({
    key,
    label: key === null ? NONE_LABEL : groupBy === "priority" ? `[#${key}]` : key,
    rows: rows.filter((_row, i) => keySets[i].includes(key)),
  }));
}

function enumValuesFor(schema: readonly FieldSpec[], field: FieldId): readonly string[] | null {
  const spec = schema.find((s) => s.field === field);
  return spec && typeof spec.type === "object" && "enum" in spec.type ? spec.type.enum : null;
}

function recordFacets(row: RowRecord): Facets | null {
  const n = doc.byId[row.id];
  if (n) return facetsOf(n.raw, formatForBlock(row.id));
  return row.dto ? facetsFromDto(row.dto) : null;
}

function moveRowToColumn(row: RowRecord, from: string | null, target: string | null, field: FieldId): boolean {
  if (field !== "tags") return writeField(row.id, field, target ?? "");
  const tags = groupKeysForBlock(row, "tags").filter((key): key is string => key !== null);
  if (from === null) return target !== null && writeTagDelta(row.id, { add: target });
  if (target === null) return tags.length === 1 && writeTagDelta(row.id, { remove: from });
  return writeTagDelta(row.id, { remove: from, add: target });
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
  const bgColor = createMemo(() => {
    const f = recordFacets(props.row);
    return f ? blockBackgroundColor(f.properties) : undefined;
  });

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
  const openCellMenu = (e: MouseEvent) => {
    if (!doc.byId[props.row.id]) return;
    e.preventDefault();
    e.stopPropagation();
    select();
    openSheetCellContextMenu(e.clientX, e.clientY, props.row.id);
  };
  const openCellMenuFromHandle = (e: MouseEvent) => {
    if (!doc.byId[props.row.id]) return;
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
      }}
      data-sheet-grid-id={props.ownerId}
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
      onContextMenu={openCellMenu}
      style={bgColor() ? { background: bgColor() } : undefined}
    >
      <Show when={doc.byId[props.row.id]}>
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
