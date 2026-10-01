import { sheetClickOffset, sheetCellMenu, displayLimitThrough } from "../sheet/interactions";
import { cellIsSelected } from "../sheet/selection";
import { clearOnBindingInvalidated } from "../binding";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, useContext, type JSX } from "solid-js";
import { blockPageReadOnly, blockProperty, blockWritable, formatForBlock, formatForPage, insertEmptyChildBlock, pageByName, readPageProperty, readPageProperties, setBlockProperty, setPageProperty, setRaw, withUndoUnit, node as docNode, pinPageWhileDrafting } from "../document";
import { facetsOf } from "../render/facets";
import { InlineText } from "../render/inline";
import { observeNear, unobserveNear } from "../lazyObserve";
import { isBuiltinHidden } from "../editor/properties";
import { isLegacyBareColumnList } from "../editor/queryViewProperties";
import { forbidsEditEntry } from "../editor/editTargets";
import { editingId, editingOwner } from "../editorController";
import { SheetCellContext, type SheetCellCtx } from "../sheet/context";
import {
  cellOwner,
  cellIsInRange,
  cellSel,
  cellSurfaceKey,
  handleCellSelectionKey,
  aggregateFooterPinned,
  clearSelectedSheetInstance,
  registerSheetViewAdapter,
  rebaseSelectedCell,
  rebaseSelectedRange,
  setCellSel,
  setAggregateFooterPinned,
  startCellEditing,
  toggleAggregateFooterPinned,
  type CellSel,
} from "../sheet/selection";
import { beginCellPointerSelection, isSheetPointerInteractive, sheetGridIdFromEventTarget } from "../sheet/pointerSelection";
import {
  cycleField,
  fieldIdsForBlocks,
  toggleStateMarkerLabel,
  fieldLabel,
  isFormulaField,
  readField,
  writeFieldVisibly,
  type FieldId,
  type FieldValue,
} from "../sheet/fields";
import { parseFields, serializeFields, sheetConfig, type FieldSpec, type FieldType } from "../sheet/config";
import { planSheetFieldRename } from "../sheet/renameField";
import { formulaFieldId, formulaNameFromField, formulasOf, mergeFormulas } from "../sheet/formulaFields";
import {
  createFormulaFilterMemo,
  createFormulaResultsMemo,
  formulaResultKey,
  formulaRowKey,
  formulaValueToFieldValue,
  readFormulaRowField,
  liveFormulaRowNode,
  type FormulaEvalRow,
} from "../sheet/formulaEval";
import type { FormulaValue } from "../sheet/formula";
import { isPlainDecimalNumber } from "../sheet/typed";
import { openActionContextMenu, openDatePicker, openFormulaEditor, openSheetContextMenu, type ContextMenuAction } from "../ui";
import { pushToast } from "../toasts";
import { blockBackgroundColor } from "../blockColors";
import type { RefGroup } from "../types";
import { Editor, SurfaceContext } from "./Block";
import { SheetAggregateCornerToggle, SheetAggregateFooterCell } from "./SheetAggregateFooter";
import { SheetContainerOverlayContext } from "./SheetContainerOverlay";
import { hydrateVisibleQueryPages, SHEET_RENDER_PAGE } from "../sheet/queryHydration";
import { compareSortKeys, measuredGridTracks, nextQuerySort, queryColumnFieldId, queryColumnName, querySortFieldName, reorderedQueryColumns,
  SCHEMA_PROP_TYPES, type SchemaMenuType, type SortKey, type SortState } from "../sheet/tablePresentation";
import { queryTableFooter, type QueryDisplayControl } from "../sheet/queryTableFooter";
import { FieldValueView } from "./SheetFieldValue";
import { displayFieldValue, isEnumFieldType } from "../sheet/cellPresentation";
import { fieldIdsForRecords, recordFacets, rowRaw, tableFieldOrder, tableRowTitle } from "../sheet/tableFields";
import { createTableColumnResize } from "../sheet/tableColumnResize";

interface RowRecord extends FormulaEvalRow {}

export const __sheetTableTestHooks: { onIndexRow?: (rowId: string) => void } = {};

type SchemaHome = { kind: "block"; id: string; value: string } | { kind: "page"; name: string; value: string };
type FormulaHome = { kind: "block"; id: string } | { kind: "page"; name: string };
type FieldHeaderDrop = { field: FieldId; before: boolean };

const BUILTIN_FIELDS = new Set<FieldId>(["state", "priority", "scheduled", "deadline", "tags", "page"]);
const FIELD_HEADER_DRAG_THRESHOLD_PX = 4;
/** Render children or query rows as a table. A query display controller owns
 * saved columns, sorts and aggregates; without it headers keep their local arrangement.
 * Resizing reads one owner's widths and writes one property through document
 * on commit; row and field work scales with the supplied table, never a graph. */
export function SheetTable(props: {
  ownerId: string;
  rowSource: "children" | "query";
  groups?: readonly RefGroup[];
  addRow?: () => void | Promise<void>;
  addRowLabel?: string;
  schemaPage?: string;
  queryDisplay?: QueryDisplayControl;
}): JSX.Element {
  const surfaceId = useContext(SurfaceContext);
  let tableRef: HTMLDivElement | undefined;
  const [sort, setSort] = createSignal<SortState>(null);
  const [extraFields, setExtraFields] = createSignal<FieldId[]>([]);
  const [addingColumn, setAddingColumn] = createSignal(false);
  const [renamingField, setRenamingField] = createSignal<{ field: FieldId; value: string } | null>(null);
  const [editingProp, setEditingProp] = createSignal<{ rowId: string; field: FieldId; initial: string } | null>(null);
  onCleanup(pinPageWhileDrafting(() => { const edit = editingProp(); return edit && docNode(edit.rowId)?.page; })); // K17a: eviction keeps the draft
  const [hovering, setHovering] = createSignal(false);
  const [stableColumns, setStableColumns] = createSignal<string | null>(null);
  const [draggingFieldHeader, setDraggingFieldHeader] = createSignal<FieldId | null>(null);
  const [fieldHeaderDrop, setFieldHeaderDrop] = createSignal<FieldHeaderDrop | null>(null);
  let sortBeforePotentialHeaderDoubleClick: SortState | undefined;
  let cancelFieldHeaderDrag: (() => void) | undefined;
  let suppressFieldHeaderClick = false;
  const sheetOverlay = useContext(SheetContainerOverlayContext);
  const sheetHovering = () => sheetOverlay?.hovering() ?? hovering();
  const config = createMemo(() => {
    const owner = docNode(props.ownerId);
    return sheetConfig(owner ? facetsOf(owner.raw, formatForBlock(props.ownerId)).properties : []);
  });
  // A pre-split bare column list on a QUERY block is the column selection, not a
  // declared schema: reading it as one marked every column stray and let the next
  // schema write replace the list (master P5A). It reads as an absent schema.
  const isSchemaValue = (value: string | null): value is string =>
    value !== null && (props.rowSource !== "query" || !isLegacyBareColumnList(value));
  const schemaHome = createMemo<SchemaHome | null>(() => {
    if (docNode(props.ownerId)) {
      const value = blockProperty(props.ownerId, "tine.fields");
      if (isSchemaValue(value)) return { kind: "block", id: props.ownerId, value };
    }
    if (props.schemaPage) {
      const value = readPageProperty(props.schemaPage, "tine.fields");
      if (isSchemaValue(value)) return { kind: "page", name: props.schemaPage, value };
    }
    return null;
  });
  const schemaFields = createMemo<readonly FieldSpec[]>(() => {
    const home = schemaHome();
    return home ? parseFields(home.value) : [];
  });
  const schemaFieldSet = createMemo(() => new Set<FieldId>(schemaFields().map((s) => s.field)));
  const fieldTypes = createMemo(() => {
    const out = new Map<FieldId, FieldType>();
    for (const spec of schemaFields()) out.set(spec.field, spec.type);
    return out;
  });
  const pageFormulas = createMemo<ReadonlyMap<string, string>>(() => {
    if (!props.schemaPage) return new Map();
    return formulasOf(readPageProperties(props.schemaPage));
  });
  const blockFormulas = createMemo<ReadonlyMap<string, string>>(() => {
    const owner = docNode(props.ownerId);
    return owner ? formulasOf(facetsOf(owner.raw, formatForBlock(props.ownerId)).properties) : new Map();
  });
  const formulas = createMemo(() => mergeFormulas(pageFormulas(), blockFormulas()));
  const formulaHomes = createMemo(() => {
    const out = new Map<string, FormulaHome>();
    if (props.schemaPage) {
      for (const name of pageFormulas().keys()) out.set(name, { kind: "page", name: props.schemaPage });
    }
    for (const name of blockFormulas().keys()) out.set(name, { kind: "block", id: props.ownerId });
    return out;
  });
  const formulaFields = createMemo<FieldId[]>(() => [...formulas().keys()].map(formulaFieldId));

  const allRows = createMemo<RowRecord[]>(() => {
    if (props.rowSource === "children") {
      return (docNode(props.ownerId)?.children ?? []).map((id) => ({
        id,
        page: docNode(id)?.page ?? docNode(props.ownerId)?.page ?? "",
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
  const formulaResults = createFormulaResultsMemo({
    rows,
    formulas,
    ownerId: props.ownerId,
  });
  const formulaValue = (row: RowRecord, field: FieldId): FormulaValue | null => {
    const name = formulaNameFromField(field);
    if (!name) return null;
    return formulaResults().get(formulaResultKey(row, name)) ?? null;
  };
  const rowFieldValue = (row: RowRecord, field: FieldId): FieldValue | null => {
    return isFormulaField(field) ? formulaValueToFieldValue(formulaValue(row, field)) : readFormulaRowField(row, field);
  };

  const fields = createMemo<FieldId[]>(() => {
    const selected = props.rowSource === "query" ? props.queryDisplay?.view.columns : undefined;
    if (selected?.length) return selected.map(queryColumnFieldId);
    const loadedIds = rows().filter((r) => liveFormulaRowNode(r)).map((r) => r.id);
    const observed = loadedIds.length === rows().length
      ? fieldIdsForBlocks(loadedIds, { includePage: props.rowSource === "query" })
      : fieldIdsForRecords(rows(), props.rowSource === "query");
    return tableFieldOrder(observed, extraFields(), schemaFields().map((s) => s.field), formulaFields());
  });
  const formulaHintFields = createMemo(() => {
    const out: string[] = [];
    const seen = new Set<string>();
    for (const field of fields()) {
      const name = formulaReferenceName(field);
      if (!name || seen.has(name)) continue;
      seen.add(name);
      out.push(name);
    }
    return out;
  });
  const formulaEntries = () => [...formulas().entries()];

  const columns = createMemo(() => ["title" as const, ...fields()]);
  const tableWidths = createTableColumnResize(props.ownerId, props.schemaPage);
  const columnIndex = createMemo(() => new Map(columns().map((column, index) => [column, index] as const)));
  const hasActionColumn = () => props.rowSource === "children" || !!props.addRow;
  const actionColumn = () => hasActionColumn() ? "96px" : "";
  // Cap column growth with fit-content() so a long cell wraps (see `.sheet-cell`
  // white-space) instead of stretching its column to the full unwrapped line —
  // an uncapped `max-content` track made one long value blow the table out
  // horizontally. Users can still resize wider (stableColumns overrides this).
  const baseGridColumns = createMemo(() => [...columns().map((column, index) =>
    tableWidths.widths().has(column) ? `${tableWidths.widths().get(column)}px`
      : index === 0 ? "fit-content(420px)" : "fit-content(320px)"), actionColumn()].join(" "));
  const editingInThisTable = () => editingOwner()?.startsWith(`sheet:${surfaceId}:${props.ownerId}:`) ?? false;
  const gridColumns = createMemo(() => {
    const stable = stableColumns();
    if (!stable || tableWidths.resizing()) return baseGridColumns();
    const tracks = stable.trim().split(/\s+/);
    columns().forEach((column, index) => {
      const width = tableWidths.widths().get(column);
      if (width !== undefined) tracks[index] = `${width}px`;
    });
    return tracks.join(" ");
  });
  const queryFooter = (field: FieldId) => queryTableFooter(props.queryDisplay, field);
  const hasAggregates = createMemo(() =>
    props.queryDisplay
      ? fields().some((field) => queryFooter(field)?.fn != null)
      : config().colAggregates.size > 0,
  );
  const footerPinned = createMemo(() => aggregateFooterPinned(props.ownerId));
  const showFooter = createMemo(() => hasAggregates() || footerPinned());
  const showFooterToggle = createMemo(() => !hasAggregates() && (sheetHovering() || footerPinned()));

  const toggleFooter = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    toggleAggregateFooterPinned(props.ownerId);
  };

  const footerToggle = () => (
    <SheetAggregateCornerToggle
      active={footerPinned()}
      onClick={toggleFooter}
    />
  );

  createEffect(() => {
    if (hasAggregates() && footerPinned()) setAggregateFooterPinned(props.ownerId, false);
  });

  createEffect(() => {
    if (!sheetOverlay) return;
    sheetOverlay.setCorner(showFooterToggle() ? footerToggle() : null);
  });

  onCleanup(() => sheetOverlay?.setCorner(null));

  const captureStableColumns = () => {
    if (!tableRef) return;
    const tracks = measuredGridTracks(tableRef, columns().length + (hasActionColumn() ? 1 : 0));
    if (tracks) setStableColumns(tracks);
  };

  let wasEditing = false;
  createEffect(() => {
    const sel = cellSel();
    baseGridColumns();
    const editing = editingInThisTable();
    if (wasEditing && !editing) {
      wasEditing = false;
      setStableColumns(null);
      return;
    }
    wasEditing = editing;
    if (!tableRef || editing) return;
    if (sel && sel.gridId === props.ownerId && sel.kind !== "row-seam" && sel.kind !== "col-seam") captureStableColumns();
    else setStableColumns(null);
  });

  const sortedRows = createMemo(() => {
    const s = sort();
    const rs = rows();
    if (!s) return rs;
    const col = columns()[s.col];
    const value = (r: RowRecord): SortKey => {
      if (col === "title") return { kind: "text", text: tableRowTitle(r) };
      const formula = formulaValue(r, col);
      if (formula?.kind === "number") return { kind: "number", value: formula.value, text: String(formula.value) };
      const field = rowFieldValue(r, col);
      const text = field?.raw ?? field?.text ?? "";
      if (fieldTypes().get(col) === "number" && isPlainDecimalNumber(text.trim())) {
        return { kind: "number", value: Number(text.trim()), text };
      }
      return { kind: "text", text };
    };
    return [...rs].sort((a, b) => compareSortKeys(value(a), value(b)) * s.dir);
  });
  const [renderLimit, setRenderLimit] = createSignal(SHEET_RENDER_PAGE);
  createEffect(() => {
    props.groups;
    setRenderLimit(SHEET_RENDER_PAGE);
  });
  const displayedRows = createMemo(() => sortedRows().slice(0, renderLimit()));
  const rowIndexes = createMemo(() => {
    const byRowKey = new Map<string, number>();
    const byBlockId = new Map<string, { row: number; rowKey: string } | null>();
    sortedRows().forEach((row, index) => {
      __sheetTableTestHooks.onIndexRow?.(row.id);
      const rowKey = formulaRowKey(row);
      byRowKey.set(rowKey, index);
      if (liveFormulaRowNode(row)) {
        if (byBlockId.has(row.id)) byBlockId.set(row.id, null);
        else byBlockId.set(row.id, { row: index, rowKey });
      }
    });
    return { byRowKey, byBlockId };
  });
  const sortedRowIndex = () => rowIndexes().byRowKey;
  const ensureDisplayedThrough = (row: number) => {
    const limit = displayLimitThrough(row, displayedRows().length, sortedRows().length, SHEET_RENDER_PAGE);
    if (limit !== null) setRenderLimit(limit);
  };
  createEffect(() => {
    const sel = cellSel();
    if (!sel || sel.gridId !== props.ownerId || sel.surfaceId !== surfaceId) return;
    if (sel.kind === "cell" && sel.rowId) {
      const row = sortedRowIndex().get(sel.rowId);
      const col = sel.columnId ? columnIndex().get(sel.columnId as "title" | FieldId) : sel.col;
      if (row !== undefined && col !== undefined) {
        ensureDisplayedThrough(row);
        rebaseSelectedCell(props.ownerId, surfaceId, sel.rowId, { row, col }, columns()[col]);
      }
      else clearSelectedSheetInstance(props.ownerId, surfaceId);
    } else if (sel.kind === "range" && sel.anchorRowId && sel.focusRowId) {
      const anchor = sortedRowIndex().get(sel.anchorRowId);
      const focus = sortedRowIndex().get(sel.focusRowId);
      const anchorCol = sel.anchorColumnId ? columnIndex().get(sel.anchorColumnId as "title" | FieldId) : sel.anchor.col;
      const focusCol = sel.focusColumnId ? columnIndex().get(sel.focusColumnId as "title" | FieldId) : sel.focus.col;
      if (anchor !== undefined && focus !== undefined && anchorCol !== undefined && focusCol !== undefined) {
        ensureDisplayedThrough(Math.max(anchor, focus));
        rebaseSelectedRange(
          props.ownerId,
          surfaceId,
          sel.anchorRowId,
          { row: anchor, col: anchorCol },
          sel.focusRowId,
          { row: focus, col: focusCol },
          columns()[anchorCol],
          columns()[focusCol]
        );
      } else clearSelectedSheetInstance(props.ownerId, surfaceId);
    }
  });

  const persistedSort = createMemo<SortState>(() => {
    const entries = props.queryDisplay?.view.sort;
    if (entries?.length !== 1) return null;
    const col = columns().findIndex((field) => querySortFieldName(field) === entries[0][0]);
    return col < 0 ? null : { col, dir: entries[0][1] === "desc" ? -1 : 1 };
  });
  createEffect(() => { if (props.queryDisplay?.view.sort) setSort(null); });
  const sortHeader = (col: number) => {
    const control = props.queryDisplay;
    const field = columns()[col];
    const name = field && querySortFieldName(field);
    if (control && name) {
      setSort(null);
      control.apply({ ...control.view, sort: nextQuerySort(control.view.sort, name) });
      return;
    }
    setSort((cur) => {
      if (!cur || cur.col !== col) return { col, dir: 1 };
      if (cur.dir === 1) return { col, dir: -1 };
      return null;
    });
  };
  const sortArrow = (col: number) => {
    const s = sort() ?? persistedSort();
    return s?.col === col ? (s.dir > 0 ? " ▲" : " ▼") : "";
  };

  const tableOnlySortLabel = () => {
    const current = sort();
    if (props.rowSource !== "query" || !current) return null;
    const field = columns()[current.col];
    return `Table-only sort: ${field === "title" ? "Title" : fieldLabel(field)}`;
  };

  const createSchemaHome = (): SchemaHome | null => {
    if (docNode(props.ownerId)) return { kind: "block", id: props.ownerId, value: "" };
    return props.schemaPage ? { kind: "page", name: props.schemaPage, value: "" } : null;
  };
  const schemaWriteAllowed = () => {
    const home = schemaHome() ?? createSchemaHome();
    if (!home) return false;
    if (home.kind === "block") return !blockPageReadOnly(home.id);
    return !(pageByName(home.name)?.readOnly ?? false);
  };
  const writeSchemaFields = (next: readonly FieldSpec[]) => {
    const home = schemaHome() ?? createSchemaHome();
    if (!home || !schemaWriteAllowed()) return;
    const value = serializeFields(next);
    // Declaring a schema writes `tine.fields`, which on a query block may still
    // hold the note's column list. Rescue it into `tine.columns` in the same undo
    // unit, only when `tine.columns` is absent (a present value always wins).
    const legacy = props.rowSource === "query" && docNode(props.ownerId)
      && blockProperty(props.ownerId, "tine.columns") === null
      && isLegacyBareColumnList(blockProperty(props.ownerId, "tine.fields"));
    const columns = legacy ? props.queryDisplay?.view.columns : undefined;
    const pages = [home.kind === "block" ? docNode(home.id)?.page : home.name, docNode(props.ownerId)?.page]
      .filter((name, i, all): name is string => !!name && all.indexOf(name) === i);
    withUndoUnit("sheet:schema-fields", pages, () => {
      if (columns && columns.length > 0) setBlockProperty(props.ownerId, "tine.columns", columns.join(";"));
      if (home.kind === "block") setBlockProperty(home.id, "tine.fields", value || null);
      else setPageProperty(home.name, "tine.fields", value || null);
    });
  };
  const formulaWriteAllowed = (home: FormulaHome | null) => {
    if (!home) return false;
    if (home.kind === "block") return !blockPageReadOnly(home.id);
    return !(pageByName(home.name)?.readOnly ?? false);
  };
  const removeFormula = (field: FieldId) => {
    const name = formulaNameFromField(field);
    if (!name) return;
    const home = formulaHomes().get(name) ?? null;
    if (!formulaWriteAllowed(home)) return;
    const key = `tine.formula.${name}`;
    if (home?.kind === "block") setBlockProperty(home.id, key, null);
    else if (home) setPageProperty(home.name, key, null);
  };
  // Start a brand-new formula column from a column header. The same command lives
  // on the table's ⋮ / body menu, but a column header is where users look for it
  // (and where the Guide points), so offer it here too.
  const addFormula = (x: number, y: number) => {
    openFormulaEditor({
      mode: "add",
      ownerId: props.ownerId,
      schemaPage: props.schemaPage,
      x,
      y,
      expr: "",
      formulas: formulaEntries(),
      fields: formulaHintFields(),
    });
  };
  const specForField = (field: FieldId, type: SchemaMenuType = "text"): FieldSpec | null => {
    if (BUILTIN_FIELDS.has(field)) return { field, type: "builtin" };
    return field.startsWith("prop:") ? { field, type } : null;
  };
  const declareField = (field: FieldId) => {
    const spec = specForField(field);
    if (!spec) return;
    writeSchemaFields([...schemaFields(), spec]);
  };
  const declareFreshSchema = () => {
    const specs = fields().map((field) => specForField(field)).filter((spec): spec is FieldSpec => !!spec);
    writeSchemaFields(specs);
  };
  const canDragFieldHeader = (field: FieldId) =>
    props.queryDisplay ? queryColumnName(field) !== null && !blockPageReadOnly(props.ownerId)
      : field.startsWith("prop:") && schemaWriteAllowed() && (!schemaHome() || schemaFieldSet().has(field));
  const canDropFieldHeader = (field: FieldId, dragged: FieldId) => {
    if (field === dragged) return false;
    if (props.queryDisplay) return queryColumnName(field) !== null;
    // Formula fields are not serialized in tine.fields. They still make a useful
    // terminal drop boundary: a property dropped on one is inserted before all
    // formulas, which are always rendered at the end.
    if (isFormulaField(field)) return true;
    return schemaHome() ? schemaFieldSet().has(field) : !!specForField(field);
  };
  const reorderFieldHeader = (field: FieldId, drop: FieldHeaderDrop) => {
    if (props.queryDisplay) {
      const columns = reorderedQueryColumns(fields(), field, drop.field, drop.before);
      if (columns) props.queryDisplay.apply({ ...props.queryDisplay.view, columns });
      else pushToast("This order cannot be saved while a computed column is visible.", "info");
      return;
    }
    if (!schemaHome()) declareFreshSchema();
    const next = [...schemaFields()];
    const from = next.findIndex((spec) => spec.field === field);
    if (from < 0) return;
    const [moved] = next.splice(from, 1);
    const target = next.findIndex((spec) => spec.field === drop.field);
    // Formula fields do not have schema specs. A drop on one means append to the
    // property schema, immediately before the pinned formula run.
    const at = target < 0 ? next.length : target + (drop.before ? 0 : 1);
    next.splice(at, 0, moved);
    writeSchemaFields(next);
  };
  const beginFieldHeaderDrag = (field: FieldId, event: PointerEvent) => {
    if (event.button !== 0 || isSheetPointerInteractive(event.target) || !canDragFieldHeader(field)) return;
    cancelFieldHeaderDrag?.();
    const pointerId = event.pointerId;
    const startX = event.clientX;
    const startY = event.clientY;
    let dragging = false;
    let active = true;

    const cleanup = () => {
      if (!active) return;
      active = false;
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      if (cancelFieldHeaderDrag === cleanup) cancelFieldHeaderDrag = undefined;
      setDraggingFieldHeader(null);
      setFieldHeaderDrop(null);
    };
    const ownsPointer = (pointer: PointerEvent) => pointer.pointerId === pointerId;
    const onMove = (move: PointerEvent) => {
      if (!ownsPointer(move)) return;
      if (!dragging && Math.hypot(move.clientX - startX, move.clientY - startY) < FIELD_HEADER_DRAG_THRESHOLD_PX) return;
      if (!dragging) {
        dragging = true;
        setDraggingFieldHeader(field);
      }
      move.preventDefault();
      const header = document.elementFromPoint(move.clientX, move.clientY)
        ?.closest<HTMLElement>("[data-sheet-field-header]");
      const target = header?.dataset.sheetField as FieldId | undefined;
      if (!header || !target || !canDropFieldHeader(target, field)) {
        setFieldHeaderDrop(null);
        return;
      }
      if (isFormulaField(target)) {
        setFieldHeaderDrop({ field: target, before: true });
        return;
      }
      const rect = header.getBoundingClientRect();
      setFieldHeaderDrop({ field: target, before: move.clientX < rect.left + rect.width / 2 });
    };
    const onCancel = (cancel: PointerEvent) => {
      if (ownsPointer(cancel)) cleanup();
    };
    const onUp = (up: PointerEvent) => {
      if (!ownsPointer(up)) return;
      const drop = fieldHeaderDrop();
      const didDrag = dragging;
      cleanup();
      if (didDrag) {
        suppressFieldHeaderClick = true;
        setTimeout(() => (suppressFieldHeaderClick = false), 0);
      }
      if (didDrag && drop) reorderFieldHeader(field, drop);
    };

    cancelFieldHeaderDrag = cleanup;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
  };
  onCleanup(() => cancelFieldHeaderDrag?.());
  const changeFieldType = (field: FieldId, type: SchemaMenuType) => {
    writeSchemaFields(schemaFields().map((spec) => (spec.field === field ? { ...spec, type } : spec)));
  };
  const removeFieldFromSchema = (field: FieldId) => {
    writeSchemaFields(schemaFields().filter((spec) => spec.field !== field));
    // Also drop it from the in-memory "added column" set — otherwise a column added
    // via "Add column" (which only lives in extraFields) lingers on screen after
    // being removed from the schema, until an app restart clears the signal.
    setExtraFields((cur) => cur.filter((f) => f !== field));
  };
  const renameDisabledReason = (field: FieldId, declared: FieldSpec | null): string | null => {
    if (props.rowSource !== "children") return "only children-backed tables can rename fields";
    if (!declared) return "declare this field first";
    if (!declared.field.startsWith("prop:")) return "built-in fields cannot be renamed";
    if (schemaHome()?.kind !== "block") return "page-inherited fields cannot be renamed here";
    if (!schemaWriteAllowed() || !blockWritable(props.ownerId)) return "this table is read-only";
    return null;
  };
  const startFieldRename = (field: FieldId, declared: FieldSpec | null) => {
    if (renameDisabledReason(field, declared)) return;
    setRenamingField({ field, value: field.slice("prop:".length) });
  };
  const commitFieldRename = (field: FieldId, value: string): boolean => {
    const owner = docNode(props.ownerId);
    const home = schemaHome();
    if (!owner || home?.kind !== "block") return false;
    const rowNodes = owner.children.map((id) => docNode(id)).filter((row): row is NonNullable<typeof row> => !!row);
    if (rowNodes.length !== owner.children.length) {
      pushToast("Some direct rows are not loaded; no fields were renamed.", "error");
      return false;
    }
    const page = props.schemaPage ? pageByName(props.schemaPage) : undefined;
    const result = planSheetFieldRename({
      rowSource: props.rowSource,
      ownerWritable: blockWritable(props.ownerId),
      schemaHome: home.kind,
      owner: {
        id: props.ownerId,
        page: owner.page,
        raw: owner.raw,
        format: formatForBlock(props.ownerId),
        recognizedProperties: facetsOf(owner.raw, formatForBlock(props.ownerId)).properties,
      },
      rows: rowNodes.map((row) => ({
        id: row.id,
        page: row.page,
        raw: row.raw,
        format: formatForBlock(row.id),
        recognizedProperties: facetsOf(row.raw, formatForBlock(row.id)).properties,
      })),
      pageProperties: page ? readPageProperties(page.name) : [],
      recognizeProperties: (raw, format) => facetsOf(raw, format).properties,
      oldField: field,
      newName: value,
    });
    if (!result.ok) {
      pushToast(result.error, "error");
      return false;
    }
    const plan = result.plan;
    withUndoUnit("sheet:rename-field", [plan.page], () => {
      setRaw(plan.ownerId, plan.ownerRaw, { timetracking: false });
      for (const row of plan.rows) setRaw(row.id, row.raw, { timetracking: false });
    });
    setExtraFields((current) => current.filter((candidate) => candidate !== plan.oldField && candidate !== plan.newField));
    setRenamingField(null);
    return true;
  };
  const openFieldHeaderMenu = (e: MouseEvent, field: FieldId) => {
    e.preventDefault();
    e.stopPropagation();
    if (isFormulaField(field)) {
      const name = formulaNameFromField(field);
      const home = name ? formulaHomes().get(name) ?? null : null;
      openActionContextMenu(e.clientX, e.clientY, [
        {
          label: props.rowSource === "children"
            ? "Rename field… (formula columns cannot be renamed here)"
            : "Rename field… (only children-backed tables can rename fields)",
          disabled: true,
        },
        {
          label: "Edit formula…",
          disabled: !name || !formulaWriteAllowed(home),
          run: () => {
            if (!name) return;
            openFormulaEditor({
              mode: "edit",
              ownerId: props.ownerId,
              schemaPage: props.schemaPage,
              x: e.clientX,
              y: e.clientY,
              name,
              expr: formulas().get(name) ?? "",
              formulas: formulaEntries(),
              fields: formulaHintFields(),
              home,
            });
          },
        },
        { label: "Remove formula", disabled: !formulaWriteAllowed(home), run: () => removeFormula(field) },
        { label: "Add formula…", disabled: !schemaWriteAllowed(), run: () => addFormula(e.clientX, e.clientY) },
      ]);
      return;
    }
    const declared = schemaFields().find((spec) => spec.field === field) ?? null;
    const disabled = !schemaWriteAllowed();
    const actions: ContextMenuAction[] = [];
    const renameReason = renameDisabledReason(field, declared);
    actions.push({
      label: renameReason ? `Rename field… (${renameReason})` : "Rename field…",
      disabled: !!renameReason,
      run: () => startFieldRename(field, declared),
    });
    if (!schemaHome()) {
      if (field.startsWith("prop:")) actions.push({ label: "Declare field (text)", disabled, run: declareFreshSchema });
    } else if (!declared) {
      actions.push({ label: "Declare field (text)", disabled, run: () => declareField(field) });
    } else if (declared.field.startsWith("prop:")) {
      actions.push({
        label: "Type →",
        disabled,
        children: SCHEMA_PROP_TYPES.map((type) => ({
          label: type,
          disabled,
          run: () => changeFieldType(field, type),
        })),
      });
      actions.push({ label: "Remove from schema", disabled, run: () => removeFieldFromSchema(field) });
    } else {
      actions.push({ label: "Remove from schema", disabled, run: () => removeFieldFromSchema(field) });
    }
    // A column added via "Add column" but never declared lives only in extraFields
    // (no schema entry to "Remove from schema"). Give it its own removal so it isn't
    // stuck on screen until restart.
    if (!declared && extraFields().includes(field)) {
      actions.push({ label: "Remove column", run: () => setExtraFields((cur) => cur.filter((f) => f !== field)) });
    }
    actions.push({ label: "Add formula…", disabled, run: () => addFormula(e.clientX, e.clientY) });
    if (actions.length) openActionContextMenu(e.clientX, e.clientY, actions);
  };

  const selected = (row: number, col: number) => cellIsSelected(props.ownerId, row, col, surfaceId);  const inRange = (row: number, col: number) => cellIsInRange(props.ownerId, row, col, surfaceId);

  const openPropInput = (rowId: string, field: FieldId, initial?: string) => {
    setEditingProp({ rowId, field, initial: initial ?? readField(rowId, field)?.text ?? "" });
  };
  const propUsesInlineInput = (field: FieldId): boolean => {
    const type = fieldTypes().get(field);
    return field.startsWith("prop:") && type !== "checkbox" && type !== "date" && type !== "datetime" && !isEnumFieldType(type);
  };
  const addChildRow = () => {
    if (props.rowSource !== "children") return;
    const owner = docNode(props.ownerId);
    if (!owner || blockPageReadOnly(props.ownerId)) return;
    const at = owner.children.length;
    const id = withUndoUnit("sheet:table-add-row", [owner.page], () => insertEmptyChildBlock(props.ownerId, at));
    if (!id) return;
    queueMicrotask(() => {
      const rowIndex = sortedRows().findIndex((row) => row.id === id && !!liveFormulaRowNode(row));
      if (rowIndex !== undefined) startCellEditing({ gridId: props.ownerId, surfaceId, rowId: id, columnId: "title", row: rowIndex, col: 0 }, 0);
    });
  };
  const runAddRow = () => {
    if (props.rowSource === "children") addChildRow();
    else void props.addRow?.();
  };

  const pointForSelection = (sel: CellSel) => {
    const row = sel.rowId ? sortedRowIndex().get(sel.rowId) : sel.row;
    const col = sel.columnId ? columnIndex().get(sel.columnId as "title" | FieldId) : sel.col;
    return row === undefined || col === undefined || row >= displayedRows().length ? null : { row, col };
  };

  const activateCell = (sel: CellSel): boolean => {
    const point = pointForSelection(sel);
    const row = point ? sortedRows()[point.row] : undefined;
    const col = point ? columns()[point.col] : undefined;
    if (!row || !col) return true;
    if (col === "title") return false;
    if (!liveFormulaRowNode(row)) return true;
    if (col === "state") return cycleField(row.id, "state");
    if (col === "priority") return cycleField(row.id, "priority");
    if (col === "scheduled" || col === "deadline") return true;
    if (propUsesInlineInput(col)) {
      openPropInput(row.id, col);
      return true;
    }
    return true;
  };

  const overtype = (sel: CellSel, text: string): boolean => {
    const point = pointForSelection(sel);
    const row = point ? sortedRows()[point.row] : undefined;
    const col = point ? columns()[point.col] : undefined;
    if (!row || !col) return true;
    if (col === "title") return false;
    if (propUsesInlineInput(col) && liveFormulaRowNode(row)) openPropInput(row.id, col, text);
    else if ((col === "scheduled" || col === "deadline") && liveFormulaRowNode(row)) writeFieldVisibly(row.id, col, text);
    return true;
  };

  onMount(() => {
    const dispose = registerSheetViewAdapter(props.ownerId, {
      bounds: () => ({ rows: displayedRows().length, cols: columns().length }),
      rowIdAt: (row) => {
        const candidate = displayedRows()[row];
        return candidate ? formulaRowKey(candidate) : null;
      },
      columnIdAt: (_row, col) => columns()[col] ?? null,
      blockIdAt: (row, col, rowId, stableColumnId) => {
        const resolvedCol = stableColumnId ? columnIndex().get(stableColumnId as "title" | FieldId) : col;
        if (resolvedCol === undefined || columns()[resolvedCol] !== "title") return null;
        if (rowId) {
          const index = sortedRowIndex().get(rowId);
          const candidate = index === undefined || index >= displayedRows().length ? null : sortedRows()[index];
          return candidate && liveFormulaRowNode(candidate) ? candidate.id : null;
        }
        const candidate = displayedRows()[row];
        return candidate && liveFormulaRowNode(candidate) ? candidate.id : null;
      },
      cellForBlock: (blockId) => {
        const match = rowIndexes().byBlockId.get(blockId);
        const row = match && match.row < displayedRows().length ? match.row : undefined;
        const col = columnIndex().get("title");
        return row !== undefined && col !== undefined
          ? { kind: "cell", gridId: props.ownerId, surfaceId, rowId: match!.rowKey, columnId: "title", row, col }
          : null;
      },
      activate: activateCell,
      overtype,
    }, surfaceId);
    onCleanup(dispose);
  });

  const addPropertyColumn = (key: string) => {
    const clean = key.trim();
    if (!clean || /[:\s]/.test(clean)) return;
    const field: FieldId = `prop:${clean}`;
    setExtraFields((cur) => (cur.includes(field) ? cur : [...cur, field]));
  };

  const openSheetMenu = (e: MouseEvent) => {
    if (!docNode(props.ownerId)) return;
    e.preventDefault();
    e.stopPropagation();
    openSheetContextMenu(e.clientX, e.clientY, props.ownerId, "table", props.rowSource, null, {
      schemaPage: props.schemaPage,
      fields: formulaHintFields(),
      formulas: formulaEntries(),
      filter: config().filter,
    });
  };
  const onPointerDown = (e: PointerEvent) => {
    if (sheetGridIdFromEventTarget(e.target) !== props.ownerId || isSheetPointerInteractive(e.target)) return;
    beginCellPointerSelection(e, props.ownerId);
  };
  const stopSheetMouseDown = (e: MouseEvent) => {
    if (e.button === 0) e.stopPropagation();
  };

  return (
    <Show
      when={rows().length > 0}
      fallback={
        <div class="sheet-table sheet-empty">
          <span>empty table</span>
          <Show when={props.rowSource === "children" || props.addRow}>
            <button
              class="sheet-add-row-ghost sheet-add-row-empty"
              title={props.addRowLabel ?? "Add row"}
              onClick={runAddRow}
            >
              <span class="sheet-ghost-sticky"><span class="sheet-ghost-plus">+</span><span>{props.addRowLabel ?? "Add row"}</span></span>
            </button>
          </Show>
        </div>
      }
    >
      <div
        ref={(el) => {
          tableRef = el;
        }}
        class="sheet-table"
        classList={{ "sheet-table-resizing": tableWidths.resizing() !== null }}
        data-sheet-grid-id={props.ownerId}
        data-sheet-surface-id={surfaceId}
        style={{ "grid-template-columns": gridColumns() }}
        onPointerDown={onPointerDown}
        onMouseDown={stopSheetMouseDown}
        onPointerEnter={() => setHovering(true)}
        onPointerLeave={() => setHovering(false)}
        onContextMenu={openSheetMenu}
      >
        <div class="sheet-cell sheet-header-cell sheet-title-header sheet-sticky-left" onClick={() => sortHeader(0)}>
          Block{sortArrow(0)}
          <Show when={tableOnlySortLabel()}>
            {(label) => <button type="button" class="sheet-table-only-sort" title="Clear table-only sort"
              onClick={(event) => { event.stopPropagation(); setSort(null); }}>{label()} ×</button>}
          </Show>
          <Show when={filterError()}>
            {(err) => (
              <span class="sheet-filter-error" title={err()}>
                Filter disabled
              </span>
            )}
          </Show>
          {tableWidths.handle("title", "Block")}
        </div>
        <For each={fields()}>
          {(field, i) => (
            <div
              class="sheet-cell sheet-header-cell sheet-field-header"
              classList={{
                "sheet-col-formula": isFormulaField(field),
                "sheet-col-stray": !!schemaHome() && !schemaFieldSet().has(field) && !isFormulaField(field),
                "sheet-header-draggable": canDragFieldHeader(field),
                "sheet-header-dragging": draggingFieldHeader() === field,
                "sheet-header-drop-before": fieldHeaderDrop()?.field === field && fieldHeaderDrop()?.before,
                "sheet-header-drop-after": fieldHeaderDrop()?.field === field && !fieldHeaderDrop()?.before,
              }}
              data-sheet-field-header
              data-sheet-field={field}
              onPointerDown={(e) => beginFieldHeaderDrag(field, e)}
              onClick={(e) => {
                if (suppressFieldHeaderClick) {
                  e.preventDefault();
                  e.stopPropagation();
                  return;
                }
                if ((e.target as HTMLElement).closest("input")) return;
                if (e.detail > 1) return;
                sortBeforePotentialHeaderDoubleClick = sort();
                sortHeader(i() + 1);
              }}
              onDblClick={(e) => {
                const declared = schemaFields().find((spec) => spec.field === field) ?? null;
                if (renameDisabledReason(field, declared)) return;
                e.preventDefault();
                e.stopPropagation();
                if (sortBeforePotentialHeaderDoubleClick !== undefined) {
                  setSort(sortBeforePotentialHeaderDoubleClick);
                  sortBeforePotentialHeaderDoubleClick = undefined;
                }
                startFieldRename(field, declared);
              }}
              onContextMenu={(e) => openFieldHeaderMenu(e, field)}
            >
              <Show
                when={renamingField()?.field === field}
                fallback={
                  <>
                    <Show when={isFormulaField(field)}>
                      <span class="sheet-formula-marker">ƒ</span>
                    </Show>
                    {fieldLabel(field)}{sortArrow(i() + 1)}
                  </>
                }
              >
                <input
                  class="sheet-prop-input sheet-header-rename-input"
                  autofocus
                  value={renamingField()?.value ?? ""}
                  aria-label={`Rename ${fieldLabel(field)} field`}
                  onClick={(e) => e.stopPropagation()}
                  onInput={(e) => setRenamingField({ field, value: e.currentTarget.value })}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === "Enter") commitFieldRename(field, e.currentTarget.value);
                    else if (e.key === "Escape") setRenamingField(null);
                  }}
                  onBlur={(e) => {
                    if (renamingField()?.field === field) commitFieldRename(field, e.currentTarget.value);
                  }}
                />
              </Show>
              {tableWidths.handle(field, fieldLabel(field))}
            </div>
          )}
        </For>
        <Show when={hasActionColumn()}>
          <div class="sheet-cell sheet-header-cell sheet-add-field">
            <Show
              when={props.rowSource === "children" && addingColumn()}
              fallback={
                <Show
                  when={props.rowSource === "children"}
                  fallback={<span class="sheet-add-column-spacer" />}
                >
                  <button
                    class="sheet-add-column-ghost"
                    title="Add column"
                    onPointerDown={(e) => e.stopPropagation()}
                    onMouseDown={(e) => e.stopPropagation()}
                    onClick={(e) => {
                      e.stopPropagation();
                      setAddingColumn(true);
                    }}
                  >
                    <span class="sheet-ghost-plus">+</span>
                    <span class="sheet-ghost-label">Add column</span>
                  </button>
                </Show>
              }
            >
              <input
                class="sheet-prop-input sheet-add-field-input"
                autofocus
                placeholder="property"
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => {
                  e.stopPropagation();
                  if (e.key === "Enter") {
                    addPropertyColumn(e.currentTarget.value);
                    setAddingColumn(false);
                  } else if (e.key === "Escape") {
                    setAddingColumn(false);
                  }
                }}
                onBlur={(e) => {
                  addPropertyColumn(e.currentTarget.value);
                  setAddingColumn(false);
                }}
              />
            </Show>
          </div>
        </Show>
        <For each={displayedRows()}>
          {(row, rowIndex) => {
            // Per-row "near the viewport" gate (see renderedSheetRows above). Only
            // the title cell observes; the shared signal drives every cell in the
            // row so we spend ONE IntersectionObserver entry per row, not per cell.
            const [near, setNear] = createSignal(renderedSheetRows.has(row.id));
            const observeRow = (el: Element) => {
              if (near()) {
                if (props.rowSource === "query") void hydrateVisibleQueryPages([row], props.groups);
                return;
              }
              observeNear(el, () => {
                renderedSheetRows.add(row.id);
                setNear(true);
                if (props.rowSource === "query") {
                  void hydrateVisibleQueryPages([row], props.groups);
                }
              });
              onCleanup(() => unobserveNear(el));
            };
            return (
              <>
                <TitleCell
                  ownerId={props.ownerId}
                  surfaceId={surfaceId}
                  row={row}
                  rowIndex={rowIndex()}
                  selected={selected(rowIndex(), 0)}
                  inRange={inRange(rowIndex(), 0)}
                  near={near()}
                  observeRow={observeRow}
                  freezeColumns={captureStableColumns}
                />
                <For each={fields()}>
                  {(field, fieldIndex) => (
                    <FieldCell
                      ownerId={props.ownerId}
                      surfaceId={surfaceId}
                      row={row}
                      field={field}
                      fieldType={fieldTypes().get(field)}
                      formulaValue={formulaValue(row, field)}
                      rowIndex={rowIndex()}
                      colIndex={fieldIndex() + 1}
                      selected={selected(rowIndex(), fieldIndex() + 1)}
                      inRange={inRange(rowIndex(), fieldIndex() + 1)}
                      near={near()}
                      editing={editingProp()?.rowId === row.id && editingProp()?.field === field}
                      initial={editingProp()?.initial ?? ""}
                      openPropInput={openPropInput}
                      closePropInput={() => setEditingProp(null)}
                      freezeColumns={captureStableColumns}
                    />
                  )}
                </For>
                <Show when={hasActionColumn()}>
                  <div class="sheet-cell sheet-row-tail" />
                </Show>
              </>
            );
          }}
        </For>
        <Show when={displayedRows().length < sortedRows().length}>
          <button
            class="sheet-add-row-ghost sheet-load-more"
            style={{ "grid-column": "1 / -1" }}
            onClick={(e) => {
              e.stopPropagation();
              setRenderLimit((limit) => limit + SHEET_RENDER_PAGE);
            }}
          >
            Load {Math.min(SHEET_RENDER_PAGE, sortedRows().length - displayedRows().length)} more rows
            ({displayedRows().length} of {sortedRows().length})
          </button>
        </Show>
        <Show when={showFooter()}>
          <div class="sheet-cell sheet-footer-cell sheet-footer-title sheet-sticky-left" />
          <For each={fields()}>
            {(field) => (
              <SheetAggregateFooterCell
                ownerId={props.ownerId}
                columnKey={field}
                fn={props.queryDisplay ? null : config().colAggregates.get(field) ?? null}
                query={queryFooter(field)}
                values={props.queryDisplay ? [] : sortedRows().map((row) => rowFieldValue(row, field))}
                showEmpty={footerPinned() && (!props.queryDisplay || queryFooter(field) !== undefined)}
              />
            )}
          </For>
          <Show when={hasActionColumn()}>
            <div class="sheet-cell sheet-footer-cell sheet-row-tail" />
          </Show>
        </Show>
        <Show when={hasActionColumn()}>
          <button
            class="sheet-add-row-ghost"
            title={props.addRowLabel ?? "Add row"}
            onPointerDown={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.stopPropagation()}
            onClick={(e) => {
              e.stopPropagation();
              runAddRow();
            }}
          >
            <span class="sheet-ghost-sticky"><span class="sheet-ghost-plus">+</span><span class="sheet-ghost-label">{props.addRowLabel ?? "Add row"}</span></span>
          </button>
        </Show>
        <Show when={!sheetOverlay && showFooterToggle()}>
          {footerToggle()}
        </Show>
      </div>
    </Show>
  );
}

function formulaReferenceName(field: FieldId): string | null {
  if (isFormulaField(field)) return null;
  if (field.startsWith("prop:")) return field.slice(5);
  return field;
}

// Lazy-mount virtualization (P2): a table row's heavy cell CONTENT (title
// InlineText parse, value-view chips, the hover handle) is deferred until the
// row first comes near the viewport, mirroring the block-body pattern
// ([[tine-block-virtualization]], lazyObserve.ts). The .sheet-cell divs, their
// data-row/col attrs, selection classes and event handlers stay mounted for
// EVERY row so selection, keyboard nav and drag hit-testing keep working
// off-screen. Render-once-keep: a row that has rendered once (latched by block
// id) renders eagerly forever — no placeholder↔real churn on re-sort/remount.
// Module-level so it survives remount and is shared across surfaces; bounded by
// the working set of rows ever brought near the viewport.
const renderedSheetRows = new Set<string>();
clearOnBindingInvalidated(() => renderedSheetRows.clear());

// Test seam: reset the render-once latch so a fresh mount defers again.
export function resetSheetRowVirtualizationForTests() {
  renderedSheetRows.clear();
}

function TitleCell(props: {
  ownerId: string;
  surfaceId: string;
  row: RowRecord;
  rowIndex: number;
  selected: boolean;
  inRange: boolean;
  near: boolean;
  observeRow: (el: Element) => void;
  freezeColumns: () => void;
}): JSX.Element {
  const cell = (): SheetCellCtx => ({
    gridId: props.ownerId,
    surfaceId: props.surfaceId,
    rowId: formulaRowKey(props.row),
    columnId: "title",
    row: props.rowIndex,
    col: 0,
  });
  let contentRef: HTMLDivElement | undefined;
  const editing = () => editingId() === props.row.id && editingOwner() === cellOwner(cell());
  const fmt = () => (liveFormulaRowNode(props.row) ? formatForBlock(props.row.id) : formatForPage(props.row.page));
  const raw = () => rowRaw(props.row);
  const bgColor = createMemo(() => {
    const f = recordFacets(props.row);
    return f ? blockBackgroundColor(f.properties) : undefined;
  });

  const onDoubleClick = (e: MouseEvent) => {
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey) return;
    if (forbidsEditEntry(e)) return;
    e.preventDefault();
    e.stopPropagation();
    props.freezeColumns();
    if (!liveFormulaRowNode(props.row)) {
      setCellSel(cell());
      return;
    }
    startCellEditing(cell(), sheetClickOffset(e, contentRef, raw(), isBuiltinHidden) ?? undefined);
  };
  const openCellMenu = (e: MouseEvent) => {
    if (!liveFormulaRowNode(props.row)) return;
    sheetCellMenu(e, () => setCellSel(cell()), props.row.id, undefined);
  };
  const openCellMenuFromHandle = (e: MouseEvent) => {
    if (!liveFormulaRowNode(props.row)) return;
    sheetCellMenu(e, () => setCellSel(cell()), props.row.id, undefined, true);
  };

  return (
    <div
      class="sheet-cell sheet-title-cell"
      classList={{
        "sheet-cell-selected": props.selected,
        "sheet-cell-in-range": props.inRange,
        "sheet-sticky-left": true,
      }}
      data-sheet-grid-id={props.ownerId}
      data-sheet-surface-id={props.surfaceId}
      data-block-id={props.row.id}
      data-sheet-row-id={formulaRowKey(props.row)}
      data-sheet-column-id="title"
      data-row={props.rowIndex}
      data-col={0}
      style={bgColor() ? { background: bgColor() } : undefined}
      ref={props.observeRow}
      onDblClick={onDoubleClick}
      onContextMenu={openCellMenu}
    >
      <Show when={props.near && liveFormulaRowNode(props.row)}>
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
      <div class="sheet-cell-body" ref={contentRef}>
        <Show
          when={editing()}
          fallback={
            <Show
              when={props.near}
              fallback={<span class="sheet-cell-defer">{tableRowTitle(props.row)}</span>}
            >
              <InlineText text={tableRowTitle(props.row)} format={fmt()} />
            </Show>
          }
        >
          <SheetCellContext.Provider value={cell()}>
            <SurfaceContext.Provider value={cellSurfaceKey(props.ownerId, props.surfaceId)}>
              <Editor id={props.row.id} />
            </SurfaceContext.Provider>
          </SheetCellContext.Provider>
        </Show>
      </div>
    </div>
  );
}

function FieldCell(props: {
  ownerId: string;
  surfaceId: string;
  row: RowRecord;
  field: FieldId;
  fieldType?: FieldType;
  formulaValue?: FormulaValue | null;
  rowIndex: number;
  colIndex: number;
  selected: boolean;
  inRange: boolean;
  near: boolean;
  editing: boolean;
  initial: string;
  openPropInput: (rowId: string, field: FieldId, initial?: string) => void;
  closePropInput: () => void;
  freezeColumns: () => void;
}): JSX.Element {
  const value = () => isFormulaField(props.field) ? formulaValueToFieldValue(props.formulaValue) : readFormulaRowField(props.row, props.field);
  const displayValue = (): FieldValue | null => displayFieldValue(props.field, props.fieldType, value());
  const editable = () => !!liveFormulaRowNode(props.row) && !isFormulaField(props.field);
  const select = () => setCellSel({
    gridId: props.ownerId,
    surfaceId: props.surfaceId,
    rowId: formulaRowKey(props.row),
    columnId: props.field,
    row: props.rowIndex,
    col: props.colIndex,
  });
  const inlinePropEditor = () =>
    props.field.startsWith("prop:") &&
    props.fieldType !== "checkbox" &&
    props.fieldType !== "date" &&
    props.fieldType !== "datetime" &&
    !isEnumFieldType(props.fieldType);
  const bgColor = createMemo(() => {
    const f = recordFacets(props.row);
    return f ? blockBackgroundColor(f.properties) : undefined;
  });
  const [inputInvalid, setInputInvalid] = createSignal(false);
  const commit = (value: string): boolean => {
    const trimmed = value.trim();
    if (props.fieldType === "number" && trimmed && !isPlainDecimalNumber(trimmed)) {
      setInputInvalid(true);
      return false;
    }
    if (editable()) writeFieldVisibly(props.row.id, props.field, value);
    props.closePropInput();
    setInputInvalid(false);
    return true;
  };
  const openEnumMenu = (e: MouseEvent, values: readonly string[]) => {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openActionContextMenu(rect.left, rect.bottom + 4, [
      ...values.map((label): ContextMenuAction => ({
        label,
        run: () => writeFieldVisibly(props.row.id, props.field, label),
      })),
      { label: "Clear", run: () => writeFieldVisibly(props.row.id, props.field, "") },
    ]);
  };

  const runControlAction = (e: MouseEvent) => {
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey) return;
    const checkboxControl = props.field.startsWith("prop:") && props.fieldType === "checkbox";
    if (!checkboxControl) e.preventDefault();
    e.stopPropagation();
    props.freezeColumns();
    select();
    if (!editable()) return;
    if (props.field === "state") toggleStateMarkerLabel(props.row.id);
    else if (props.field === "priority") cycleField(props.row.id, "priority");
    else if (props.field === "scheduled" || props.field === "deadline") {
      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
      openDatePicker(props.row.id, props.field, rect.left, rect.bottom + 4);
    }
    else if (props.field.startsWith("prop:")) {
      const type = props.fieldType;
      if (type === "checkbox") {
        const cur = (value()?.raw ?? value()?.text ?? "").trim().toLowerCase();
        writeFieldVisibly(props.row.id, props.field, cur === "true" ? "false" : "true");
      } else if (type === "date" || type === "datetime") {
        const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
        openDatePicker(props.row.id, { field: props.field as `prop:${string}`, fieldType: type }, rect.left, rect.bottom + 4);
      } else if (isEnumFieldType(type)) {
        openEnumMenu(e, type.enum);
      }
    }
  };

  const isDateField = () =>
    props.field === "scheduled" ||
    props.field === "deadline" ||
    (props.field.startsWith("prop:") && (props.fieldType === "date" || props.fieldType === "datetime"));
  // A date cell with no value renders nothing to click (the chip is the only
  // control), so there's no way to ADD a date to a row that lacks one (Martin's
  // nit). Let a single click anywhere in an EMPTY date cell open the picker; a
  // filled cell keeps its chip handler (which stops propagation, so this no-ops).
  const onCellClick = (e: MouseEvent) => {
    if (!isDateField() || !editable()) return;
    if ((e.target as HTMLElement).closest(".date-chip")) return; // chip already handles it
    runControlAction(e);
  };

  const onDoubleClick = (e: MouseEvent) => {
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey) return;
    e.preventDefault();
    e.stopPropagation();
    props.freezeColumns();
    select();
    if (editable() && inlinePropEditor()) props.openPropInput(props.row.id, props.field);
  };
  const openCellMenu = (e: MouseEvent) => {
    if (!editable()) return;
    sheetCellMenu(e, select, props.row.id, { rowId: props.row.id });
  };
  const openCellMenuFromHandle = (e: MouseEvent) => {
    if (!editable()) return;
    sheetCellMenu(e, select, props.row.id, { rowId: props.row.id }, true);
  };

  return (
    <div
      class="sheet-cell sheet-field-cell"
      classList={{
        "sheet-cell-selected": props.selected,
        "sheet-cell-in-range": props.inRange,
        "sheet-readonly-cell": !editable() || props.field === "tags" || props.field === "page" || isFormulaField(props.field),
        "sheet-number-cell":
          (props.field.startsWith("prop:") && props.fieldType === "number") ||
          (isFormulaField(props.field) && props.formulaValue?.kind === "number"),
      }}
      data-sheet-grid-id={props.ownerId}
      data-sheet-surface-id={props.surfaceId}
      data-sheet-row-id={formulaRowKey(props.row)}
      data-sheet-column-id={props.field}
      data-block-id={props.row.id}
      data-row={props.rowIndex}
      data-col={props.colIndex}
      style={bgColor() ? { background: bgColor() } : undefined}
      onClick={onCellClick}
      onDblClick={onDoubleClick}
      onContextMenu={openCellMenu}
    >
      <Show when={props.near && editable()}>
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
      <Show
        when={props.editing && props.field.startsWith("prop:")}
        fallback={
          <Show
            when={props.near}
            fallback={<span class="sheet-cell-defer">{displayValue()?.text ?? ""}</span>}
          >
            <FieldValueView
              field={props.field}
              fieldType={props.fieldType}
              value={displayValue()}
              formulaValue={props.formulaValue}
              page={props.row.page}
              onControlClick={runControlAction}
            />
          </Show>
        }
      >
        <input
          class="sheet-prop-input"
          classList={{ "sheet-input-invalid": inputInvalid() }}
          autofocus
          ref={(el) => {
            // Dynamically inserted autofocus inputs are not focused reliably by
            // WebKit. Without an explicit handoff, type-to-overtype displays
            // its first character but the next Tab is handled from selection
            // mode and discards that draft (GH #176).
            queueMicrotask(() => {
              if (!el.isConnected) return;
              el.focus();
              const end = el.value.length;
              el.setSelectionRange(end, end);
            });
          }}
          value={props.initial}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          onInput={(e) => {
            if (props.fieldType !== "number") return;
            const trimmed = e.currentTarget.value.trim();
            setInputInvalid(!!trimmed && !isPlainDecimalNumber(trimmed));
          }}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") commit(e.currentTarget.value);
            else if (!e.ctrlKey && !e.metaKey && !e.altKey && (e.key === "Tab" || e.code === "Tab")) {
              // A native input's Tab never reaches the global sheet-selection
              // handler. Commit first, then advance through the same stable
              // row/column path used by selected cells. Otherwise blur saves
              // this draft but leaves selection behind, so the next typed
              // value overtypes this same cell (GH #176).
              if (commit(e.currentTarget.value)) handleCellSelectionKey(e);
              e.preventDefault();
            } else if (e.key === "Escape") {
              setInputInvalid(false);
              props.closePropInput();
            }
          }}
          onBlur={(e) => {
            // e.currentTarget is null once dispatch ends — capture before the microtask
            const el = e.currentTarget;
            if (!commit(el.value)) queueMicrotask(() => el.focus());
          }}
        />
      </Show>
    </div>
  );
}
