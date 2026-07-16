import { For, Match, Show, Switch, createEffect, createMemo, createSignal, onCleanup, onMount, useContext, type JSX } from "solid-js";
import {
  blockPageReadOnly,
  blockProperty,
  blockWritable,
  doc,
  formatForBlock,
  formatForPage,
  insertEmptyChildBlock,
  pageByName,
  readPageProperty,
  setBlockProperty,
  setPageProperty,
  setRaw,
  withUndoUnit,
} from "../store";
import { facetsFromDto, facetsOf, type Facets } from "../render/facets";
import { pageProperties, visibleBody, isRenderHiddenProp } from "../render/block";
import { InlineText } from "../render/inline";
import { observeNear, unobserveNear } from "../lazyObserve";
import { editorOffsetFromRenderedRange } from "../render/spans";
import { isBuiltinHidden } from "../editor/properties";
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
  fieldLabel,
  isFormulaField,
  readField,
  writeField,
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
  formulaValueText,
  formulaValueToFieldValue,
  readFormulaRowField,
  liveFormulaRowNode,
  type FormulaEvalRow,
} from "../sheet/formulaEval";
import type { FormulaValue } from "../sheet/formula";
import { isPlainDecimalNumber, parseIsoDateLike } from "../sheet/typed";
import {
  openActionContextMenu,
  openDatePicker,
  openFormulaEditor,
  openSheetCellContextMenu,
  openSheetContextMenu,
  pushToast,
  type ContextMenuAction,
} from "../ui";
import { blockBackgroundColor } from "../blockColors";
import type { RefGroup } from "../types";
import { Editor, SurfaceContext } from "./Block";
import { SheetAggregateCornerToggle, SheetAggregateFooterCell } from "./SheetAggregateFooter";
import { SheetContainerOverlayContext } from "./SheetContainerOverlay";
import { hydrateVisibleQueryPages, SHEET_RENDER_PAGE } from "../sheet/queryHydration";

interface RowRecord extends FormulaEvalRow {}

export const __sheetTableTestHooks: { onIndexRow?: (rowId: string) => void } = {};

type SortState = { col: number; dir: 1 | -1 } | null;
type SortKey = { kind: "number"; value: number; text: string } | { kind: "text"; text: string };
type SchemaHome = { kind: "block"; id: string; value: string } | { kind: "page"; name: string; value: string };
type FormulaHome = { kind: "block"; id: string } | { kind: "page"; name: string };
type SchemaMenuType = "text" | "number" | "date" | "datetime" | "checkbox" | "list" | "ref";

const BUILTIN_FIELDS = new Set<FieldId>(["state", "priority", "scheduled", "deadline", "tags", "page"]);
const SCHEMA_PROP_TYPES: SchemaMenuType[] = [
  "text",
  "number",
  "date",
  "datetime",
  "checkbox",
  "list",
  "ref",
];

function measuredGridTracks(grid: HTMLElement, count: number): string | null {
  const cells = [...grid.children].filter((child): child is HTMLElement =>
    child instanceof HTMLElement && child.classList.contains("sheet-cell")
  );
  const tracks: string[] = [];
  for (const cell of cells.slice(0, count)) {
    const width = cell.getBoundingClientRect().width;
    if (width <= 0) return null;
    tracks.push(`${Math.round(width)}px`);
  }
  return tracks.length === count ? tracks.join(" ") : null;
}

function compareSortKeys(a: SortKey, b: SortKey): number {
  if (a.kind === "number" && b.kind === "number") return a.value - b.value;
  return a.text.localeCompare(b.text);
}

export function SheetTable(props: {
  ownerId: string;
  rowSource: "children" | "query";
  groups?: readonly RefGroup[];
  addRow?: () => void | Promise<void>;
  addRowLabel?: string;
  schemaPage?: string;
}): JSX.Element {
  const surfaceId = useContext(SurfaceContext);
  let tableRef: HTMLDivElement | undefined;
  const [sort, setSort] = createSignal<SortState>(null);
  const [extraFields, setExtraFields] = createSignal<FieldId[]>([]);
  const [addingColumn, setAddingColumn] = createSignal(false);
  const [renamingField, setRenamingField] = createSignal<{ field: FieldId; value: string } | null>(null);
  const [editingProp, setEditingProp] = createSignal<{ rowId: string; field: FieldId; initial: string } | null>(null);
  const [hovering, setHovering] = createSignal(false);
  const [stableColumns, setStableColumns] = createSignal<string | null>(null);
  let sortBeforePotentialHeaderDoubleClick: SortState | undefined;
  const sheetOverlay = useContext(SheetContainerOverlayContext);
  const sheetHovering = () => sheetOverlay?.hovering() ?? hovering();
  const config = createMemo(() => {
    const owner = doc.byId[props.ownerId];
    return sheetConfig(owner ? facetsOf(owner.raw, formatForBlock(props.ownerId)).properties : []);
  });
  const schemaHome = createMemo<SchemaHome | null>(() => {
    if (doc.byId[props.ownerId]) {
      const value = blockProperty(props.ownerId, "tine.fields");
      if (value !== null) return { kind: "block", id: props.ownerId, value };
    }
    if (props.schemaPage) {
      const value = readPageProperty(props.schemaPage, "tine.fields");
      if (value !== null) return { kind: "page", name: props.schemaPage, value };
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
    const page = pageByName(props.schemaPage);
    return page ? formulasOf(pageProperties(page.preBlock, page.format)) : new Map();
  });
  const blockFormulas = createMemo<ReadonlyMap<string, string>>(() => {
    const owner = doc.byId[props.ownerId];
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
  const formulaFieldSet = createMemo(() => new Set<FieldId>(formulaFields()));

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
    const loadedIds = rows().filter((r) => liveFormulaRowNode(r)).map((r) => r.id);
    const observed = loadedIds.length === rows().length
      ? fieldIdsForBlocks(loadedIds, { includePage: props.rowSource === "query" })
      : fieldIdsForRecords(rows(), props.rowSource === "query");
    const seen = new Set(observed);
    const extra = extraFields().filter((f) => !seen.has(f));
    const inferred = [...observed, ...extra];
    const schema = schemaFields();
    const declared = schema.map((s) => s.field);
    const declaredSet = new Set(declared);
    const formulas = formulaFields();
    const formulasSet = formulaFieldSet();
    return [
      ...declared,
      ...formulas,
      ...inferred.filter((f) => !declaredSet.has(f) && !formulasSet.has(f)),
    ];
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
  const columnIndex = createMemo(() => new Map(columns().map((column, index) => [column, index] as const)));
  const hasActionColumn = () => props.rowSource === "children" || !!props.addRow;
  const actionColumn = () => hasActionColumn() ? "96px" : "";
  // Cap column growth with fit-content() so a long cell wraps (see `.sheet-cell`
  // white-space) instead of stretching its column to the full unwrapped line —
  // an uncapped `max-content` track made one long value blow the table out
  // horizontally. Users can still resize wider (stableColumns overrides this).
  const baseGridColumns = createMemo(() =>
    // NB: fit-content() is NOT valid inside minmax() — that voids the whole
    // grid-template-columns and collapses the table to one column. Use bare
    // fit-content() tracks; the title cell keeps its own min-width (`.sheet-cell`
    // / `.sheet-title-header`), so the 180px floor is preserved.
    `fit-content(420px) repeat(${fields().length}, fit-content(320px)) ${actionColumn()}`
  );
  const editingInThisTable = () => editingOwner()?.startsWith(`sheet:${surfaceId}:${props.ownerId}:`) ?? false;
  const gridColumns = createMemo(() => stableColumns() ?? baseGridColumns());
  const hasAggregates = createMemo(() => config().colAggregates.size > 0);
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
      if (col === "title") return { kind: "text", text: rowTitle(r) };
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
    if (row < 0 || row < displayedRows().length) return;
    setRenderLimit(Math.min(sortedRows().length, Math.ceil((row + 1) / SHEET_RENDER_PAGE) * SHEET_RENDER_PAGE));
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

  const sortHeader = (col: number) => {
    setSort((cur) => {
      if (!cur || cur.col !== col) return { col, dir: 1 };
      if (cur.dir === 1) return { col, dir: -1 };
      return null;
    });
  };
  const sortArrow = (col: number) => {
    const s = sort();
    return s?.col === col ? (s.dir > 0 ? " ▲" : " ▼") : "";
  };

  const createSchemaHome = (): SchemaHome | null => {
    if (doc.byId[props.ownerId]) return { kind: "block", id: props.ownerId, value: "" };
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
    if (home.kind === "block") setBlockProperty(home.id, "tine.fields", value || null);
    else setPageProperty(home.name, "tine.fields", value || null);
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
    const owner = doc.byId[props.ownerId];
    const home = schemaHome();
    if (!owner || home?.kind !== "block") return false;
    const rowNodes = owner.children.map((id) => doc.byId[id]).filter((row): row is NonNullable<typeof row> => !!row);
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
      pageProperties: page ? pageProperties(page.preBlock, page.format) : [],
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

  const selected = (row: number, col: number) => {
    const sel = cellSel();
    if (!sel || sel.gridId !== props.ownerId || (sel.surfaceId && sel.surfaceId !== surfaceId)) return false;
    if (sel.kind === "cell") return sel.row === row && sel.col === col;
    if (sel.kind === "range") return sel.focus.row === row && sel.focus.col === col;
    return false;
  };
  const inRange = (row: number, col: number) => cellIsInRange(props.ownerId, row, col, surfaceId);

  const openPropInput = (rowId: string, field: FieldId, initial?: string) => {
    setEditingProp({ rowId, field, initial: initial ?? readField(rowId, field)?.text ?? "" });
  };
  const propUsesInlineInput = (field: FieldId): boolean => {
    const type = fieldTypes().get(field);
    return field.startsWith("prop:") && type !== "checkbox" && type !== "date" && type !== "datetime" && !isEnumFieldType(type);
  };
  const addChildRow = () => {
    if (props.rowSource !== "children") return;
    const owner = doc.byId[props.ownerId];
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
    else if ((col === "scheduled" || col === "deadline") && liveFormulaRowNode(row)) writeField(row.id, col, text);
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
    if (!doc.byId[props.ownerId]) return;
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
              <span class="sheet-ghost-plus">+</span>
              <span>{props.addRowLabel ?? "Add row"}</span>
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
          <Show when={filterError()}>
            {(err) => (
              <span class="sheet-filter-error" title={err()}>
                Filter disabled
              </span>
            )}
          </Show>
        </div>
        <For each={fields()}>
          {(field, i) => (
            <div
              class="sheet-cell sheet-header-cell sheet-field-header"
              classList={{
                "sheet-col-formula": isFormulaField(field),
                "sheet-col-stray": !!schemaHome() && !schemaFieldSet().has(field) && !isFormulaField(field),
              }}
              onClick={(e) => {
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
                fn={config().colAggregates.get(field) ?? null}
                values={sortedRows().map((row) => rowFieldValue(row, field))}
                showEmpty={footerPinned()}
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
            <span class="sheet-ghost-plus">+</span>
            <span class="sheet-ghost-label">{props.addRowLabel ?? "Add row"}</span>
          </button>
        </Show>
        <Show when={!sheetOverlay && showFooterToggle()}>
          {footerToggle()}
        </Show>
      </div>
    </Show>
  );
}

function recordFacets(row: RowRecord): Facets | null {
  const n = liveFormulaRowNode(row);
  if (n) return facetsOf(n.raw, formatForBlock(row.id));
  return row.dto ? facetsFromDto(row.dto) : null;
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

function rowRaw(row: RowRecord): string {
  return liveFormulaRowNode(row)?.raw ?? row.dto?.raw ?? "";
}

function rowTitle(row: RowRecord): string {
  const title = visibleBody(rowRaw(row)).join(" ");
  return title.trim() === "" && (liveFormulaRowNode(row)?.children.length ?? row.dto?.children.length ?? 0) > 0 ? "—" : title;
}

function clickOffset(e: MouseEvent, contentRef: HTMLDivElement | undefined, raw: string): number | null {
  if (!contentRef) return null;
  const d = document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
  const range = d.caretRangeFromPoint?.(e.clientX, e.clientY);
  if (!range) return null;
  return editorOffsetFromRenderedRange(contentRef, range, raw, isBuiltinHidden);
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
    startCellEditing(cell(), clickOffset(e, contentRef, raw()) ?? undefined);
  };
  const openCellMenu = (e: MouseEvent) => {
    if (!liveFormulaRowNode(props.row)) return;
    e.preventDefault();
    e.stopPropagation();
    setCellSel(cell());
    openSheetCellContextMenu(e.clientX, e.clientY, props.row.id);
  };
  const openCellMenuFromHandle = (e: MouseEvent) => {
    if (!liveFormulaRowNode(props.row)) return;
    e.preventDefault();
    e.stopPropagation();
    setCellSel(cell());
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openSheetCellContextMenu(rect.right, rect.bottom + 2, props.row.id);
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
              fallback={<span class="sheet-cell-defer">{rowTitle(props.row)}</span>}
            >
              <InlineText text={rowTitle(props.row)} format={fmt()} />
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
  const displayValue = (): FieldValue | null => {
    const current = value();
    if (current) return current;
    return props.field.startsWith("prop:") && props.fieldType === "checkbox" ? { text: "false", raw: "false" } : null;
  };
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
    if (editable()) writeField(props.row.id, props.field, value);
    props.closePropInput();
    setInputInvalid(false);
    return true;
  };
  const openEnumMenu = (e: MouseEvent, values: readonly string[]) => {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openActionContextMenu(rect.left, rect.bottom + 4, [
      ...values.map((label): ContextMenuAction => ({
        label,
        run: () => writeField(props.row.id, props.field, label),
      })),
      { label: "Clear", run: () => writeField(props.row.id, props.field, "") },
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
    if (props.field === "state") cycleField(props.row.id, "state");
    else if (props.field === "priority") cycleField(props.row.id, "priority");
    else if (props.field === "scheduled" || props.field === "deadline") {
      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
      openDatePicker(props.row.id, props.field, rect.left, rect.bottom + 4);
    }
    else if (props.field.startsWith("prop:")) {
      const type = props.fieldType;
      if (type === "checkbox") {
        const cur = (value()?.raw ?? value()?.text ?? "").trim().toLowerCase();
        writeField(props.row.id, props.field, cur === "true" ? "false" : "true");
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
    e.preventDefault();
    e.stopPropagation();
    select();
    openSheetCellContextMenu(e.clientX, e.clientY, props.row.id, { rowId: props.row.id });
  };
  const openCellMenuFromHandle = (e: MouseEvent) => {
    if (!editable()) return;
    e.preventDefault();
    e.stopPropagation();
    select();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openSheetCellContextMenu(rect.right, rect.bottom + 2, props.row.id, { rowId: props.row.id });
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

function FieldValueView(props: {
  field: FieldId;
  fieldType?: FieldType;
  value: FieldValue | null;
  formulaValue?: FormulaValue | null;
  page: string;
  onControlClick?: (e: MouseEvent) => void;
}): JSX.Element {
  if (isFormulaField(props.field)) return <FormulaValueView value={props.formulaValue ?? null} />;
  const text = () => props.value?.text ?? "";
  const stopControlDoubleClick = (e: MouseEvent) => {
    if (!props.onControlClick) return;
    e.preventDefault();
    e.stopPropagation();
  };
  return (
    <Show when={props.value}>
      <Show when={props.field === "state"}>
        <span
          class={`block-marker marker-${(props.value?.raw ?? "").toLowerCase()}`}
          onClick={props.onControlClick}
          onDblClick={stopControlDoubleClick}
        >
          {props.value?.text}
        </span>
      </Show>
      <Show when={props.field === "priority"}>
        <span
          class={`block-priority priority-${props.value?.raw}`}
          onClick={props.onControlClick}
          onDblClick={stopControlDoubleClick}
        >
          {props.value?.text}
        </span>
      </Show>
      <Show when={props.field === "scheduled"}>
        <span class="date-chip scheduled" onClick={props.onControlClick} onDblClick={stopControlDoubleClick}>{text()}</span>
      </Show>
      <Show when={props.field === "deadline"}>
        <span class="date-chip deadline" onClick={props.onControlClick} onDblClick={stopControlDoubleClick}>{text()}</span>
      </Show>
      <Show when={props.field === "tags"}>
        <For each={(props.value?.raw ?? "").split(/\s+/).filter(Boolean)}>
          {(tag) => <span class="sheet-tag-chip">#{tag}</span>}
        </For>
      </Show>
      <Show when={props.field.startsWith("prop:")}>
        <PropValueView type={props.fieldType} value={props.value!} page={props.page} onControlClick={props.onControlClick} />
      </Show>
      <Show when={props.field === "page"}>
        <InlineText text={text()} format={formatForPage(props.page)} />
      </Show>
    </Show>
  );
}

function FormulaValueView(props: { value: FormulaValue | null }): JSX.Element {
  return (
    <Switch>
      <Match when={props.value?.kind === "error"}>
        <span class="sheet-formula-error" title={props.value?.kind === "error" ? props.value.message : ""}>
          ⚠
        </span>
      </Match>
      <Match when={props.value?.kind === "number"}>
        {formulaValueText(props.value)}
      </Match>
      <Match when={props.value?.kind === "date"}>
        <span class="date-chip scheduled">{formulaValueText(props.value)}</span>
      </Match>
      <Match when={props.value?.kind === "boolean"}>
        <input
          class="sheet-checkbox"
          type="checkbox"
          checked={props.value?.kind === "boolean" ? props.value.value : false}
          disabled
        />
      </Match>
      <Match when={props.value?.kind === "list"}>
        <For each={props.value?.kind === "list" ? props.value.values : []}>
          {(value) => <span class="sheet-tag-chip">{formulaValueText(value)}</span>}
        </For>
      </Match>
      <Match when={props.value?.kind === "text" || props.value?.kind === "duration"}>
        {formulaValueText(props.value)}
      </Match>
    </Switch>
  );
}

function PropValueView(props: { type?: FieldType; value: FieldValue; page: string; onControlClick?: (e: MouseEvent) => void }): JSX.Element {
  const text = () => props.value.text;
  const raw = () => props.value.raw ?? props.value.text;
  const stopControlDoubleClick = (e: MouseEvent) => {
    if (!props.onControlClick) return;
    e.preventDefault();
    e.stopPropagation();
  };
  const checkbox = () => {
    if (props.type !== "checkbox") return null;
    const lower = raw().trim().toLowerCase();
    if (lower === "true") return true;
    if (lower === "false") return false;
    return null;
  };
  const dateValue = () => {
    if (props.type !== "date" && props.type !== "datetime") return null;
    const value = raw().trim();
    return validDateLike(value) ? value : null;
  };
  const enumValue = () => {
    if (!isEnumFieldType(props.type)) return null;
    const value = raw().trim();
    return props.type.enum.includes(value) ? value : null;
  };
  const listValues = () =>
    props.type === "list"
      ? raw()
          .split(",")
          .map((v) => v.trim())
          .filter(Boolean)
      : [];
  const refValue = () => {
    if (props.type !== "ref") return null;
    const value = raw().trim();
    return /^\[\[[^\]\n\r]+\]\]$/.test(value) ? value : null;
  };
  return (
    <Switch fallback={<InlineText text={text()} format={formatForPage(props.page)} />}>
      <Match when={checkbox() !== null}>
        <input
          class="sheet-checkbox"
          type="checkbox"
          checked={checkbox() === true}
          readOnly
          onClick={props.onControlClick}
          onDblClick={stopControlDoubleClick}
        />
      </Match>
      <Match when={dateValue()}>
        {(value) => <span class="date-chip scheduled" onClick={props.onControlClick} onDblClick={stopControlDoubleClick}>{value()}</span>}
      </Match>
      <Match when={enumValue()}>
        {(value) => <span class="sheet-tag-chip" onClick={props.onControlClick} onDblClick={stopControlDoubleClick}>{value()}</span>}
      </Match>
      <Match when={props.type === "list" && listValues().length > 0}>
        <For each={listValues()}>{(value) => <span class="sheet-tag-chip">{value}</span>}</For>
      </Match>
      <Match when={refValue()}>
        {(value) => <InlineText text={value()} format={formatForPage(props.page)} />}
      </Match>
    </Switch>
  );
}

function isEnumFieldType(type: FieldType | undefined): type is { enum: readonly string[] } {
  return typeof type === "object" && type !== null && "enum" in type;
}

function validDateLike(value: string): boolean {
  return parseIsoDateLike(value) !== null;
}
