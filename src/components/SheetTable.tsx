import { For, Match, Show, Switch, createMemo, createResource, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import {
  blockPageReadOnly,
  blockProperty,
  doc,
  ensurePageLoaded,
  formatForBlock,
  formatForPage,
  insertEmptyChildBlock,
  pageByName,
  readPageProperty,
  setBlockProperty,
  setPageProperty,
  withUndoUnit,
} from "../store";
import { facetsFromDto, facetsOf, type Facets } from "../render/facets";
import { visibleBody, isRenderHiddenProp } from "../render/block";
import { InlineText } from "../render/inline";
import { editorOffsetFromRenderedRange } from "../render/spans";
import { isBuiltinHidden } from "../editor/properties";
import { forbidsEditEntry } from "../editor/editTargets";
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
  cycleField,
  fieldIdsForBlocks,
  fieldLabel,
  readField,
  writeField,
  type FieldId,
  type FieldValue,
} from "../sheet/fields";
import { parseFields, serializeFields, sheetConfig, type FieldSpec, type FieldType } from "../sheet/config";
import { isPlainDecimalNumber } from "../sheet/typed";
import { openActionContextMenu, openDatePicker, openSheetCellContextMenu, openSheetContextMenu, type ContextMenuAction } from "../ui";
import { blockBackgroundColor } from "../blockColors";
import type { BlockDto, RefGroup } from "../types";
import { Editor, SurfaceContext } from "./Block";
import { SheetAggregateFooterCell } from "./SheetAggregateFooter";

interface RowRecord {
  id: string;
  page: string;
  dto?: BlockDto;
}

type SortState = { col: number; dir: 1 | -1 } | null;
type SchemaHome = { kind: "block"; id: string; value: string } | { kind: "page"; name: string; value: string };
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

export function SheetTable(props: {
  ownerId: string;
  rowSource: "children" | "query";
  groups?: readonly RefGroup[];
  addRow?: () => void | Promise<void>;
  addRowLabel?: string;
  schemaPage?: string;
}): JSX.Element {
  const [sort, setSort] = createSignal<SortState>(null);
  const [extraFields, setExtraFields] = createSignal<FieldId[]>([]);
  const [addingColumn, setAddingColumn] = createSignal(false);
  const [editingProp, setEditingProp] = createSignal<{ rowId: string; field: FieldId; initial: string } | null>(null);
  const [hovering, setHovering] = createSignal(false);
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

  const fields = createMemo<FieldId[]>(() => {
    const loadedIds = rows().filter((r) => doc.byId[r.id]).map((r) => r.id);
    const observed = loadedIds.length === rows().length
      ? fieldIdsForBlocks(loadedIds, { includePage: props.rowSource === "query" })
      : fieldIdsForRecords(rows(), props.rowSource === "query");
    const seen = new Set(observed);
    const extra = extraFields().filter((f) => !seen.has(f));
    const inferred = [...observed, ...extra];
    const schema = schemaFields();
    if (schema.length === 0) return inferred;
    const declared = schema.map((s) => s.field);
    const declaredSet = new Set(declared);
    return [...declared, ...inferred.filter((f) => !declaredSet.has(f))];
  });

  const columns = createMemo(() => ["title" as const, ...fields()]);
  const hasActionColumn = () => props.rowSource === "children" || !!props.addRow;
  const actionColumn = () => props.rowSource === "children" ? "58px" : hasActionColumn() ? "34px" : "";
  const gridColumns = createMemo(() =>
    `minmax(180px, max-content) repeat(${fields().length}, max-content) ${actionColumn()}`
  );
  const hasAggregates = createMemo(() => config().colAggregates.size > 0);

  const sortedRows = createMemo(() => {
    const s = sort();
    const rs = rows();
    if (!s) return rs;
    const col = columns()[s.col];
    const value = (r: RowRecord) =>
      col === "title" ? rowTitle(r) : fieldValue(r, col)?.text ?? "";
    return [...rs].sort((a, b) => value(a).localeCompare(value(b)) * s.dir);
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
  };
  const openFieldHeaderMenu = (e: MouseEvent, field: FieldId) => {
    e.preventDefault();
    e.stopPropagation();
    const declared = schemaFields().find((spec) => spec.field === field) ?? null;
    const disabled = !schemaWriteAllowed();
    const actions: ContextMenuAction[] = [];
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
    if (actions.length) openActionContextMenu(e.clientX, e.clientY, actions);
  };

  const selected = (row: number, col: number) => {
    const sel = cellSel();
    if (!sel || sel.gridId !== props.ownerId) return false;
    if (sel.kind === "cell") return sel.row === row && sel.col === col;
    if (sel.kind === "range") return sel.focus.row === row && sel.focus.col === col;
    return false;
  };

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
      const rowIndex = sortedRows().findIndex((row) => row.id === id);
      if (rowIndex >= 0) startCellEditing({ gridId: props.ownerId, row: rowIndex, col: 0 }, 0);
    });
  };
  const runAddRow = () => {
    if (props.rowSource === "children") addChildRow();
    else void props.addRow?.();
  };

  const activateCell = (sel: CellSel): boolean => {
    const row = sortedRows()[sel.row];
    const col = columns()[sel.col];
    if (!row || !col) return true;
    if (col === "title") return false;
    if (!doc.byId[row.id]) return true;
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
    const row = sortedRows()[sel.row];
    const col = columns()[sel.col];
    if (!row || !col) return true;
    if (col === "title") return false;
    if (propUsesInlineInput(col) && doc.byId[row.id]) openPropInput(row.id, col, text);
    else if ((col === "scheduled" || col === "deadline") && doc.byId[row.id]) writeField(row.id, col, text);
    return true;
  };

  onMount(() => {
    const dispose = registerSheetViewAdapter(props.ownerId, {
      bounds: () => ({ rows: sortedRows().length, cols: columns().length }),
      blockIdAt: (row, col) => (columns()[col] === "title" ? sortedRows()[row]?.id ?? null : null),
      activate: activateCell,
      overtype,
    });
    onCleanup(dispose);
  });

  const addPropertyColumn = (key: string) => {
    const clean = key.trim();
    if (!clean || /[:\s]/.test(clean)) return;
    const field: FieldId = `prop:${clean}`;
    setExtraFields((cur) => (cur.includes(field) ? cur : [...cur, field]));
  };

  const openSheetMenu = (e: MouseEvent) => {
    if (props.rowSource !== "children") return;
    e.preventDefault();
    e.stopPropagation();
    openSheetContextMenu(e.clientX, e.clientY, props.ownerId, "table", props.rowSource);
  };

  return (
    <Show
      when={rows().length > 0}
      fallback={
        <div class="sheet-table sheet-empty">
          <span>empty table</span>
          <Show when={props.rowSource === "children" || props.addRow}>
            <button
              class="sheet-add-field-btn sheet-add-row-btn"
              title={props.addRowLabel ?? "Add row"}
              onClick={runAddRow}
            />
          </Show>
        </div>
      }
    >
      <div
        class="sheet-table"
        data-sheet-grid-id={props.ownerId}
        style={{ "grid-template-columns": gridColumns() }}
        onPointerEnter={() => setHovering(true)}
        onPointerLeave={() => setHovering(false)}
        onContextMenu={openSheetMenu}
      >
        <div class="sheet-cell sheet-header-cell sheet-title-header" onClick={() => sortHeader(0)}>
          Block{sortArrow(0)}
        </div>
        <For each={fields()}>
          {(field, i) => (
            <div
              class="sheet-cell sheet-header-cell sheet-field-header"
              classList={{ "sheet-col-stray": !!schemaHome() && !schemaFieldSet().has(field) }}
              onClick={() => sortHeader(i() + 1)}
              onContextMenu={(e) => openFieldHeaderMenu(e, field)}
            >
              {fieldLabel(field)}{sortArrow(i() + 1)}
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
                  fallback={
                    <button
                      class="sheet-add-field-btn sheet-add-row-btn"
                      title={props.addRowLabel ?? "Add row"}
                      onClick={(e) => {
                        e.stopPropagation();
                        runAddRow();
                      }}
                    />
                  }
                >
                  <div class="sheet-header-actions">
                    <button
                      class="sheet-add-field-btn"
                      title="Add property column"
                      onClick={(e) => {
                        e.stopPropagation();
                        setAddingColumn(true);
                      }}
                    >
                      +
                    </button>
                    <button
                      class="sheet-add-field-btn sheet-add-row-btn"
                      title={props.addRowLabel ?? "Add row"}
                      onClick={(e) => {
                        e.stopPropagation();
                        runAddRow();
                      }}
                    />
                  </div>
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
        <For each={sortedRows()}>
          {(row, rowIndex) => (
            <>
              <TitleCell
                ownerId={props.ownerId}
                row={row}
                rowIndex={rowIndex()}
                selected={selected(rowIndex(), 0)}
              />
              <For each={fields()}>
                {(field, fieldIndex) => (
                  <FieldCell
                    ownerId={props.ownerId}
                    row={row}
                    field={field}
                    fieldType={fieldTypes().get(field)}
                    rowIndex={rowIndex()}
                    colIndex={fieldIndex() + 1}
                    selected={selected(rowIndex(), fieldIndex() + 1)}
                    editing={editingProp()?.rowId === row.id && editingProp()?.field === field}
                    initial={editingProp()?.initial ?? ""}
                    openPropInput={openPropInput}
                    closePropInput={() => setEditingProp(null)}
                  />
                )}
              </For>
              <Show when={hasActionColumn()}>
                <div class="sheet-cell sheet-row-tail" />
              </Show>
            </>
          )}
        </For>
        <Show when={hasAggregates()}>
          <div class="sheet-cell sheet-footer-cell sheet-footer-title" />
          <For each={fields()}>
            {(field) => (
              <SheetAggregateFooterCell
                ownerId={props.ownerId}
                columnKey={field}
                fn={config().colAggregates.get(field) ?? null}
                values={sortedRows().map((row) => fieldValue(row, field))}
                showEmpty={hovering()}
              />
            )}
          </For>
          <Show when={hasActionColumn()}>
            <div class="sheet-cell sheet-footer-cell sheet-row-tail" />
          </Show>
        </Show>
        <Show when={!hasAggregates() && hovering()}>
          <div class="sheet-footer-overlay" style={{ "grid-template-columns": gridColumns() }}>
            <div class="sheet-cell sheet-footer-cell sheet-footer-title" />
            <For each={fields()}>
              {(field) => (
                <SheetAggregateFooterCell
                  ownerId={props.ownerId}
                  columnKey={field}
                  fn={null}
                  values={sortedRows().map((row) => fieldValue(row, field))}
                  showEmpty
                />
              )}
            </For>
            <Show when={hasActionColumn()}>
              <div class="sheet-cell sheet-footer-cell sheet-row-tail" />
            </Show>
          </div>
        </Show>
      </div>
    </Show>
  );
}

function recordFacets(row: RowRecord): Facets | null {
  const n = doc.byId[row.id];
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

function fieldValue(row: RowRecord, field: FieldId): FieldValue | null {
  if (doc.byId[row.id]) return readField(row.id, field);
  const f = recordFacets(row);
  if (!f) return null;
  switch (field) {
    case "state":
      return f.marker ? { text: f.marker, raw: f.marker } : null;
    case "priority":
      return f.priority ? { text: `[#${f.priority}]`, raw: f.priority } : null;
    case "scheduled":
      return f.scheduled ? { text: f.scheduled, raw: f.scheduled } : null;
    case "deadline":
      return f.deadline ? { text: f.deadline, raw: f.deadline } : null;
    case "tags":
      return f.tags.length ? { text: f.tags.map((t) => `#${t}`).join(" "), raw: f.tags.join(" ") } : null;
    case "page":
      return { text: row.page, raw: row.page };
    default: {
      const key = field.slice(5);
      const prop = f.properties.find(([k]) => k === key);
      return prop ? { text: prop[1], raw: prop[1] } : null;
    }
  }
}

function rowRaw(row: RowRecord): string {
  return doc.byId[row.id]?.raw ?? row.dto?.raw ?? "";
}

function rowTitle(row: RowRecord): string {
  return visibleBody(rowRaw(row)).join(" ");
}

function clickOffset(e: MouseEvent, contentRef: HTMLDivElement | undefined, raw: string): number | null {
  if (!contentRef) return null;
  const d = document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
  const range = d.caretRangeFromPoint?.(e.clientX, e.clientY);
  if (!range) return null;
  return editorOffsetFromRenderedRange(contentRef, range, raw, isBuiltinHidden);
}

function TitleCell(props: { ownerId: string; row: RowRecord; rowIndex: number; selected: boolean }): JSX.Element {
  const cell = (): SheetCellCtx => ({ gridId: props.ownerId, row: props.rowIndex, col: 0 });
  let contentRef: HTMLDivElement | undefined;
  const editing = () => editingId() === props.row.id && editingOwner() === cellOwner(cell());
  const fmt = () => (doc.byId[props.row.id] ? formatForBlock(props.row.id) : formatForPage(props.row.page));
  const raw = () => rowRaw(props.row);
  const bgColor = createMemo(() => {
    const f = recordFacets(props.row);
    return f ? blockBackgroundColor(f.properties) : undefined;
  });

  const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
    if (forbidsEditEntry(e)) return;
    e.preventDefault();
    e.stopPropagation();
    if (!doc.byId[props.row.id]) {
      setCellSel(cell());
      return;
    }
    startCellEditing(cell(), clickOffset(e, contentRef, raw()) ?? undefined);
  };
  const openCellMenu = (e: MouseEvent) => {
    if (!doc.byId[props.row.id]) return;
    e.preventDefault();
    e.stopPropagation();
    setCellSel(cell());
    openSheetCellContextMenu(e.clientX, e.clientY, props.row.id);
  };
  const openCellMenuFromHandle = (e: MouseEvent) => {
    if (!doc.byId[props.row.id]) return;
    e.preventDefault();
    e.stopPropagation();
    setCellSel(cell());
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openSheetCellContextMenu(rect.right, rect.bottom + 2, props.row.id);
  };

  return (
    <div
      class="sheet-cell sheet-title-cell"
      classList={{ "sheet-cell-selected": props.selected }}
      data-sheet-grid-id={props.ownerId}
      data-block-id={props.row.id}
      data-row={props.rowIndex}
      data-col={0}
      style={bgColor() ? { background: bgColor() } : undefined}
      onMouseDown={onMouseDown}
      onContextMenu={openCellMenu}
    >
      <Show when={doc.byId[props.row.id]}>
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
      <div class="sheet-cell-body" ref={contentRef}>
        <Show
          when={editing()}
          fallback={<InlineText text={rowTitle(props.row)} format={fmt()} />}
        >
          <SheetCellContext.Provider value={cell()}>
            <SurfaceContext.Provider value={cellSurfaceKey(props.ownerId)}>
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
  row: RowRecord;
  field: FieldId;
  fieldType?: FieldType;
  rowIndex: number;
  colIndex: number;
  selected: boolean;
  editing: boolean;
  initial: string;
  openPropInput: (rowId: string, field: FieldId, initial?: string) => void;
  closePropInput: () => void;
}): JSX.Element {
  const value = () => fieldValue(props.row, props.field);
  const editable = () => !!doc.byId[props.row.id];
  const select = () => setCellSel({ gridId: props.ownerId, row: props.rowIndex, col: props.colIndex });
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

  const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
    e.preventDefault();
    e.stopPropagation();
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
      } else {
        props.openPropInput(props.row.id, props.field);
      }
    }
  };
  const openCellMenu = (e: MouseEvent) => {
    if (!editable()) return;
    e.preventDefault();
    e.stopPropagation();
    select();
    openSheetCellContextMenu(e.clientX, e.clientY, props.row.id);
  };
  const openCellMenuFromHandle = (e: MouseEvent) => {
    if (!editable()) return;
    e.preventDefault();
    e.stopPropagation();
    select();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openSheetCellContextMenu(rect.right, rect.bottom + 2, props.row.id);
  };

  return (
    <div
      class="sheet-cell sheet-field-cell"
      classList={{
        "sheet-cell-selected": props.selected,
        "sheet-readonly-cell": !editable() || props.field === "tags" || props.field === "page",
        "sheet-number-cell": props.field.startsWith("prop:") && props.fieldType === "number",
      }}
      data-sheet-grid-id={props.ownerId}
      data-block-id={props.row.id}
      data-row={props.rowIndex}
      data-col={props.colIndex}
      style={bgColor() ? { background: bgColor() } : undefined}
      onMouseDown={onMouseDown}
      onContextMenu={openCellMenu}
    >
      <Show when={editable()}>
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
      <Show
        when={props.editing && props.field.startsWith("prop:")}
        fallback={<FieldValueView field={props.field} fieldType={props.fieldType} value={value()} page={props.row.page} />}
      >
        <input
          class="sheet-prop-input"
          classList={{ "sheet-input-invalid": inputInvalid() }}
          autofocus
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
            else if (e.key === "Escape") {
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

function FieldValueView(props: { field: FieldId; fieldType?: FieldType; value: FieldValue | null; page: string }): JSX.Element {
  const text = () => props.value?.text ?? "";
  return (
    <Show when={props.value}>
      <Show when={props.field === "state"}>
        <span class={`block-marker marker-${(props.value?.raw ?? "").toLowerCase()}`}>{props.value?.text}</span>
      </Show>
      <Show when={props.field === "priority"}>
        <span class={`block-priority priority-${props.value?.raw}`}>{props.value?.text}</span>
      </Show>
      <Show when={props.field === "scheduled"}>
        <span class="date-chip scheduled">{text()}</span>
      </Show>
      <Show when={props.field === "deadline"}>
        <span class="date-chip deadline">{text()}</span>
      </Show>
      <Show when={props.field === "tags"}>
        <For each={(props.value?.raw ?? "").split(/\s+/).filter(Boolean)}>
          {(tag) => <span class="sheet-tag-chip">#{tag}</span>}
        </For>
      </Show>
      <Show when={props.field.startsWith("prop:")}>
        <PropValueView type={props.fieldType} value={props.value!} page={props.page} />
      </Show>
      <Show when={props.field === "page"}>
        <InlineText text={text()} format={formatForPage(props.page)} />
      </Show>
    </Show>
  );
}

function PropValueView(props: { type?: FieldType; value: FieldValue; page: string }): JSX.Element {
  const text = () => props.value.text;
  const raw = () => props.value.raw ?? props.value.text;
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
        <input class="sheet-checkbox" type="checkbox" checked={checkbox() === true} disabled />
      </Match>
      <Match when={dateValue()}>
        {(value) => <span class="date-chip scheduled">{value()}</span>}
      </Match>
      <Match when={enumValue()}>
        {(value) => <span class="sheet-tag-chip">{value()}</span>}
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
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2}))?$/.exec(value);
  if (!m) return false;
  const y = Number(m[1]);
  const mo = Number(m[2]);
  const d = Number(m[3]);
  const hh = m[4] == null ? 0 : Number(m[4]);
  const mm = m[5] == null ? 0 : Number(m[5]);
  if (mo < 1 || mo > 12 || hh > 23 || mm > 59) return false;
  const dt = new Date(Date.UTC(y, mo - 1, d, hh, mm));
  return dt.getUTCFullYear() === y && dt.getUTCMonth() === mo - 1 && dt.getUTCDate() === d;
}
