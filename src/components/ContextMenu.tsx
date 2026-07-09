import { For, Show, Switch, Match, createSignal, type JSX } from "solid-js";
import {
  contextMenu,
  closeContextMenu,
  zoomInto,
  openBlockInSidebar,
  openPageInSidebar,
  isFavorite,
  toggleFavorite,
  pushToast,
  graphMeta,
  setJournalTemplate,
  openPageProps,
  openExportModal,
  openPdfExport,
  openFormulaEditor,
  type ContextMenuAction,
  type SheetCellRemoveCtx,
} from "../ui";
import { openPage, openPageInNewTab, openPageAtBlock } from "../router";
import { closePane, layoutPaneIds, paneRouter } from "../panes";
import { refreshAfterRename } from "../graph";
import { backend } from "../backend";
import { carryDay } from "../carry";
import { journalTitle } from "../journal";
import { BLOCK_COLOR_NAMES, BLOCK_COLOR_SWATCH } from "../blockColors";
import {
  doc,
  ensureBlockId,
  persistentBlockRef,
  blockSubtreeMarkdown,
  deleteBlock,
  setBlockProperty,
  toggleBlockProperty,
  blockProperty,
  setHeading,
  setCollapsedDeep,
  dtoSubtreeMarkdown,
  flushAll,
  deletePage,
  restoreTodayJournalInFeed,
  selectedIds,
} from "../store";
import { canFlatten, flatten, hierarchify } from "../sheet/restructure";
import { canConvertPipeTableToGrid, convertGridToPipeTable, convertPipeTableToGrid } from "../sheet/conversions";
import { appendSheetCellChild, deleteColumn, setBoardGroupBy } from "../sheet/mutations";
import { cellForBlockId, cellOwner, setCellSel } from "../sheet/selection";
import { boardGroupByOptions, fieldIdsForBlocks, fieldLabel, isFieldId, type FieldId } from "../sheet/fields";
import { startEditing } from "../editorController";
import { copyStripCollapsed } from "../copySettings";
import { copyOutline } from "../clipboard";
import type { PageKind } from "../types";

// Copy a block reference/embed — but only after the block's id:: is durably on
// disk. ensureBlockId returns null if the save couldn't land (conflict/error), in
// which case we must NOT copy a ref that would dangle after a restart.
async function copyBlockRef(id: string, fmt: (uuid: string) => string, okMsg: string) {
  const uuid = await ensureBlockId(id);
  if (!uuid) {
    pushToast("Couldn't save the block id — reference not copied (resolve the conflict first).", "error");
    return;
  }
  await backend().writeText(fmt(uuid));
  pushToast(okMsg, "success");
}

// Right-click context menu. Universal over its target: a block (full editing
// menu — colors, headings, open/copy/cut, collapse, numbered list) or a page
// reference (open / open in sidebar / new tab / copy ref). The target is
// whatever you right-clicked, so right-clicking a [[page]] acts on the page,
// not the block that contains it.
export function ContextMenu(): JSX.Element {
  const close = () => closeContextMenu();

  return (
    <Show when={contextMenu()}>
      {(m) => (
        <div class="ctx-overlay" onClick={close} onContextMenu={(e) => { e.preventDefault(); close(); }}>
          <div
            class="ctx-menu"
            style={{ left: `${m().x}px`, top: `${m().y}px` }}
            onClick={(e) => e.stopPropagation()}
          >
            <Switch>
              <Match when={m().kind === "block"}>
                <BlockMenu id={(m() as { blockId: string }).blockId} close={close} />
              </Match>
              <Match when={m().kind === "blockref"}>
                <BlockRefMenu
                  uuid={(m() as { uuid: string }).uuid}
                  page={(m() as { page: string }).page}
                  pageKind={(m() as { pageKind: "journal" | "page" }).pageKind}
                  close={close}
                />
              </Match>
              <Match when={m().kind === "sheet-cell"}>
                <SheetCellMenu
                  id={(m() as { blockId: string }).blockId}
                  remove={(m() as { remove?: SheetCellRemoveCtx }).remove}
                  close={close}
                />
              </Match>
              <Match when={m().kind === "page"}>
                <PageMenu
                  name={(m() as { name: string }).name}
                  pageKind={(m() as { pageKind: "journal" | "page" }).pageKind}
                  x={m().x}
                  y={m().y}
                  close={close}
                />
              </Match>
              <Match when={m().kind === "sheet"}>
                <SheetMenu
                  ownerId={(m() as { ownerId: string }).ownerId}
                  surface={(m() as { surface: "grid" | "table" | "board" }).surface}
                  rowSource={(m() as { rowSource: "children" | "query" }).rowSource}
                  groupBy={(m() as { groupBy?: string | null }).groupBy}
                  schemaPage={(m() as { schemaPage?: string }).schemaPage}
                  fields={(m() as { fields?: readonly string[] }).fields}
                  formulas={(m() as { formulas?: readonly [string, string][] }).formulas}
                  filter={(m() as { filter?: string | null }).filter}
                  x={m().x}
                  y={m().y}
                  close={close}
                />
              </Match>
              <Match when={m().kind === "action-menu"}>
                <ActionMenu items={(m() as { items: readonly ContextMenuAction[] }).items} close={close} />
              </Match>
            </Switch>
          </div>
        </div>
      )}
    </Show>
  );
}

function ActionMenu(props: { items: readonly ContextMenuAction[]; close: () => void }): JSX.Element {
  const run = (item: ContextMenuAction) => {
    if (item.disabled) return;
    item.run?.();
    props.close();
  };

  return (
    <For each={props.items}>
      {(item) => (
        <Show
          when={item.children?.length}
          fallback={
            <div
              class="ctx-item"
              classList={{ "ctx-disabled": !!item.disabled, danger: !!item.danger }}
              onClick={() => run(item)}
            >
              {item.label}
            </div>
          }
        >
          <div class="ctx-item ctx-submenu" classList={{ "ctx-disabled": !!item.disabled }}>
            <span>{item.label}</span>
            <div class="ctx-submenu-menu">
              <For each={item.children ?? []}>
                {(child) => (
                  <div
                    class="ctx-item"
                    classList={{ "ctx-disabled": !!child.disabled, danger: !!child.danger }}
                    onClick={() => run(child)}
                  >
                    {child.label}
                  </div>
                )}
              </For>
            </div>
          </div>
        </Show>
      )}
    </For>
  );
}

function BlockMenu(props: { id: string; close: () => void }): JSX.Element {
  return (
    <>
      {/* Color row */}
      <ColorPalette id={props.id} close={props.close} />

      {/* Heading row */}
      <div class="ctx-row ctx-headings">
        <For each={[1, 2, 3, 4, 5, 6]}>
          {(h) => (
            <button class="ctx-h" title={`Heading ${h}`} onClick={() => { setHeading(props.id, h); props.close(); }}>
              H{h}
            </button>
          )}
        </For>
        <button class="ctx-h" title="Remove heading" onClick={() => { setHeading(props.id, null); props.close(); }}>
          ⌫
        </button>
      </div>

      <div class="ctx-sep" />

      <For each={blockActions(props.id)}>
        {(it) => (
          <div
            class="ctx-item"
            classList={{ danger: !!it.danger }}
            onClick={() => { it.run(); props.close(); }}
          >
            {it.label}
          </div>
        )}
      </For>

      <div class="ctx-sep" />
      <MakeTemplate id={props.id} close={props.close} />
    </>
  );
}

function ColorPalette(props: { id: string; close: () => void }): JSX.Element {
  return (
    <div class="ctx-row ctx-colors">
      <button
        class="ctx-color ctx-color-none"
        title="No background"
        onClick={() => { setBlockProperty(props.id, "background-color", null); props.close(); }}
      >
        ✕
      </button>
      <For each={BLOCK_COLOR_NAMES}>
        {(c) => (
          <button
            class="ctx-color"
            title={c}
            style={{ background: BLOCK_COLOR_SWATCH[c] }}
            onClick={() => { toggleBlockProperty(props.id, "background-color", c); props.close(); }}
          />
        )}
      </For>
    </div>
  );
}

function SheetCellMenu(props: { id: string; remove?: SheetCellRemoveCtx; close: () => void }): JSX.Element {
  const view = () => blockProperty(props.id, "tine.view") ?? "outline";
  const canDeleteRow = () => !!props.remove?.rowId && !!doc.byId[props.remove.rowId];
  const canDeleteColumn = () =>
    props.remove?.gridId != null && props.remove?.col != null && !!doc.byId[props.remove.gridId];
  const deleteRow = () => {
    const rowId = props.remove?.rowId;
    if (rowId && doc.byId[rowId]) deleteBlock(rowId);
    props.close();
  };
  const deleteColumnHere = () => {
    const { gridId, col } = props.remove ?? {};
    if (gridId != null && col != null) deleteColumn(gridId, col);
    props.close();
  };
  const setView = (next: "outline" | "grid" | "table") => {
    setBlockProperty(props.id, "tine.view", next === "outline" ? null : next);
    props.close();
  };
  const addChild = () => {
    const sel = cellForBlockId(props.id);
    const child = appendSheetCellChild(props.id);
    if (child) {
      if (sel) {
        setCellSel(sel);
        startEditing(child, 0, cellOwner(sel));
      } else {
        startEditing(child, 0);
      }
    }
    props.close();
  };
  const label = (name: string, active: boolean) => `${active ? "✓ " : ""}${name}`;

  return (
    <>
      <ColorPalette id={props.id} close={props.close} />
      <div class="ctx-sep" />
      <div class="ctx-item ctx-submenu">
        <span>Show children as →</span>
        <div class="ctx-submenu-menu">
          <div class="ctx-item" onClick={() => setView("outline")}>
            {label("Outline", view() === "outline")}
          </div>
          <div class="ctx-item" onClick={() => setView("grid")}>
            {label("Grid", view() === "grid")}
          </div>
          <div class="ctx-item" onClick={() => setView("table")}>
            {label("Table", view() === "table")}
          </div>
        </div>
      </div>
      <div
        class="ctx-item"
        onClick={addChild}
      >
        Add child bullet
      </div>
      <div
        class="ctx-item"
        onClick={() => {
          zoomInto(props.id);
          props.close();
        }}
      >
        Zoom into cell
      </div>
      <Show when={canDeleteRow() || canDeleteColumn()}>
        <div class="ctx-sep" />
        <Show when={canDeleteRow()}>
          <div class="ctx-item danger" onClick={deleteRow}>
            Delete row
          </div>
        </Show>
        <Show when={canDeleteColumn()}>
          <div class="ctx-item danger" onClick={deleteColumnHere}>
            Delete column
          </div>
        </Show>
      </Show>
    </>
  );
}

function sheetFields(ownerId: string): FieldId[] {
  return fieldIdsForBlocks(doc.byId[ownerId]?.children ?? []).filter(
    (field): field is FieldId => field === "state" || field === "priority" || field.startsWith("prop:")
  );
}

function SheetMenu(props: {
  ownerId: string;
  surface: "grid" | "table" | "board";
  rowSource: "children" | "query";
  groupBy?: string | null;
  schemaPage?: string;
  fields?: readonly string[];
  formulas?: readonly [string, string][];
  filter?: string | null;
  x: number;
  y: number;
  close: () => void;
}): JSX.Element {
  const fields = () => sheetFields(props.ownerId);
  const formulaFields = () => props.fields ?? fields().map(formulaReferenceName).filter((v): v is string => !!v);
  const formulaActions = () => props.surface === "table" || props.surface === "board";
  const doHierarchify = (field: FieldId) => {
    hierarchify(props.ownerId, field);
    props.close();
  };
  const doFlatten = () => {
    flatten(props.ownerId);
    props.close();
  };
  const boardField = () => (props.groupBy && isFieldId(props.groupBy) ? props.groupBy : null);
  const boardGroupField = (): FieldId => {
    const raw = props.groupBy || "state";
    const normalized = raw.startsWith("formula.") ? `formula:${raw.slice("formula.".length)}` : raw;
    return isFieldId(normalized) ? normalized : "state";
  };
  const doGroupBy = (field: FieldId) => {
    setBoardGroupBy(props.ownerId, field);
    props.close();
  };

  return (
    <>
      <div
        class="ctx-item"
        onClick={() => {
          zoomInto(props.ownerId);
          props.close();
        }}
      >
        Open as full page
      </div>
      <div class="ctx-sep" />
      <Show when={formulaActions()}>
        <div
          class="ctx-item"
          onClick={() => {
            openFormulaEditor({
              mode: "add",
              ownerId: props.ownerId,
              schemaPage: props.schemaPage,
              x: props.x,
              y: props.y,
              expr: "",
              formulas: props.formulas ?? [],
              fields: formulaFields(),
            });
            props.close();
          }}
        >
          Add formula…
        </div>
        <div
          class="ctx-item"
          onClick={() => {
            openFormulaEditor({
              mode: "filter",
              ownerId: props.ownerId,
              schemaPage: props.schemaPage,
              x: props.x,
              y: props.y,
              expr: props.filter ?? "",
              formulas: props.formulas ?? [],
              fields: formulaFields(),
            });
            props.close();
          }}
        >
          Edit filter…
        </div>
        <div class="ctx-sep" />
      </Show>
      <Show when={props.surface === "board"}>
        <div class="ctx-item ctx-submenu">
          <span>Group by →</span>
          <div class="ctx-submenu-menu">
            <For each={boardGroupByOptions(props.ownerId)}>
              {(field) => (
                <div
                  class="ctx-item"
                  classList={{ "ctx-active": field === boardGroupField() }}
                  onClick={() => doGroupBy(field)}
                >
                  {field === boardGroupField() ? "✓ " : ""}
                  {fieldLabel(field)}
                </div>
              )}
            </For>
          </div>
        </div>
        <div class="ctx-sep" />
      </Show>
      <Show when={props.rowSource === "children"} fallback={<div class="ctx-item ctx-disabled">No structural actions</div>}>
      <Show when={props.surface === "grid"}>
        <div
          class="ctx-item"
          onClick={() => {
            convertGridToPipeTable(props.ownerId);
            props.close();
          }}
        >
          Convert to pipe table
        </div>
        <div class="ctx-sep" />
      </Show>
      <Show when={props.surface === "board" && boardField()}>
        {(field) => (
          <div class="ctx-item" onClick={() => doHierarchify(field())}>
            Hierarchify into columns
          </div>
        )}
      </Show>
      <Show
        when={fields().length > 0}
        fallback={<div class="ctx-item ctx-disabled">Hierarchify by →</div>}
      >
        <div class="ctx-item ctx-submenu">
          <span>Hierarchify by →</span>
          <div class="ctx-submenu-menu">
            <For each={fields()}>
              {(field) => (
                <div class="ctx-item" onClick={() => doHierarchify(field)}>
                  {fieldLabel(field)}
                </div>
              )}
            </For>
          </div>
        </div>
      </Show>
      <div
        class="ctx-item"
        classList={{ "ctx-disabled": !canFlatten(props.ownerId) }}
        onClick={() => {
          if (canFlatten(props.ownerId)) doFlatten();
        }}
      >
        Flatten
      </div>
      </Show>
    </>
  );
}

function formulaReferenceName(field: FieldId): string | null {
  if (field.startsWith("formula:")) return null;
  if (field.startsWith("prop:")) return field.slice(5);
  return field;
}

// Right-click menu for an INLINE block ref `((uuid))` — acts on the referenced
// (target) block: open it in the sidebar, jump to it, or copy a ref/embed. (OG's
// menu also has delete/replace, which edit the containing block's text — those are
// just a normal edit of the block here, so they're left off this menu.)
function BlockRefMenu(props: {
  uuid: string;
  page: string;
  pageKind: "journal" | "page";
  close: () => void;
}): JSX.Element {
  const items = [
    {
      label: "Open in sidebar",
      run: () => openBlockInSidebar({ uuid: props.uuid, page: props.page, pageKind: props.pageKind }),
    },
    { label: "Go to block", run: () => openPageAtBlock(props.page, props.pageKind, props.uuid) },
    {
      label: "Copy block ref",
      run: () => { void backend().writeText(`((${props.uuid}))`); pushToast("Copied block ref", "success"); },
    },
    {
      label: "Copy block embed",
      run: () => { void backend().writeText(`{{embed ((${props.uuid}))}}`); pushToast("Copied block embed", "success"); },
    },
  ];
  return (
    <For each={items}>
      {(it) => (
        <div class="ctx-item" onClick={() => { it.run(); props.close(); }}>
          {it.label}
        </div>
      )}
    </For>
  );
}

// "Make a template" — mirrors OG Logseq: a context-menu action that expands into
// an inline name field (+ an "Include parent block" toggle when the block has
// children), and on submit marks the block with `template:: <name>`. The block
// stays where it is; that property is the template. Insert it later via `/<name>`.
function MakeTemplate(props: { id: string; close: () => void }): JSX.Element {
  const [editing, setEditing] = createSignal(false);
  const [name, setName] = createSignal("");
  // OG default: the toggle starts ON when the block has children — the named
  // block is inserted together with its children. Off → `template-including-parent::
  // false` (only the children are inserted; the block is just the template's label).
  const [includeParent, setIncludeParent] = createSignal(true);
  const hasChildren = () => (doc.byId[props.id]?.children.length ?? 0) > 0;

  const submit = async () => {
    const title = name().trim();
    if (!title) return;
    const existing = await backend().listTemplates().catch(() => []);
    if (existing.some((t) => t.name.toLowerCase() === title.toLowerCase())) {
      pushToast(`A template named “${title}” already exists.`, "error");
      return;
    }
    setBlockProperty(props.id, "template", title);
    if (hasChildren() && !includeParent()) {
      setBlockProperty(props.id, "template-including-parent", "false");
    }
    pushToast(`Template “${title}” created.`, "success");
    props.close();
  };

  return (
    <Show
      when={editing()}
      fallback={
        <div class="ctx-item" onClick={(e) => { e.stopPropagation(); setEditing(true); }}>
          Make a template…
        </div>
      }
    >
      <div class="ctx-template-form">
        <input
          class="ctx-template-name"
          placeholder="Template name"
          autofocus
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") { e.preventDefault(); void submit(); }
            else if (e.key === "Escape") { e.preventDefault(); setEditing(false); }
          }}
        />
        <Show when={hasChildren()}>
          <label class="ctx-template-toggle">
            <input
              type="checkbox"
              checked={includeParent()}
              onChange={(e) => setIncludeParent(e.currentTarget.checked)}
            />
            Include parent block
          </label>
        </Show>
        <button class="ctx-template-submit" onClick={() => void submit()}>
          Create template
        </button>
        <div class="ctx-template-hint">
          Tip: in the template, <code>{"<% today %>"}</code>, <code>{"<% current page %>"}</code>,{" "}
          <code>{"<% date: +3d %>"}</code> expand when it's inserted (or via <code>/Template var</code>).
        </div>
      </div>
    </Show>
  );
}

function PageMenu(props: {
  name: string;
  pageKind: PageKind;
  x: number;
  y: number;
  close: () => void;
}): JSX.Element {
  const fav = () => isFavorite(props.name);
  const remove = async () => {
    // Snapshot props BEFORE any await/close: the menu's <Show> disposes this
    // component the instant props.close() runs, after which reading props.* warns
    // "stale read from <Show>".
    const name = props.name;
    const kind = props.pageKind;
    // Native GTK confirm — window.confirm silently returns true here, which would
    // delete the page with no prompt.
    if (!(await backend().confirm(`Delete "${name}"? The file moves to the graph's .tine-trash folder.`))) return;
    // Route through the store (not backend directly) so it tombstones the page and
    // cancels any pending save — otherwise a just-typed, never-saved page could be
    // recreated by a queued save right after we delete it.
    void deletePage(name, kind)
      .then((ok) => {
        if (!ok) {
          pushToast("Delete failed", "error");
          return;
        }
        for (const paneId of layoutPaneIds()) {
          const router = paneRouter(paneId);
          const r = router.route();
          if (r.kind !== "page" || r.name !== name) continue;
          if (router.canGoBack()) router.goBack();
          else if (!closePane(paneId)) router.openJournals({ inPlace: true });
        }
        // Deleted a day IN the journals feed (in place, no navigation) → the feed
        // loader's withToday didn't re-run, so restore today's empty placeholder
        // here if it was the one deleted (#17). No-op for an older day.
        if (kind === "journal") restoreTodayJournalInFeed();
        pushToast(`Deleted “${name}”`, "success");
      })
      .catch(() => pushToast("Delete failed", "error"));
  };
  const items: { label: string; run: () => void; danger?: boolean }[] = [
    { label: "Open", run: () => openPage(props.name, props.pageKind) },
    { label: "Open in sidebar", run: () => openPageInSidebar(props.name, props.pageKind) },
    { label: "Open in new tab", run: () => openPageInNewTab(props.name, props.pageKind) },
    { label: fav() ? "Remove from favorites" : "Add to favorites", run: () => toggleFavorite(props.name, props.pageKind) },
    { label: "Copy page ref", run: () => { void backend().writeText(`[[${props.name}]]`); pushToast("Copied page ref", "success"); } },
    {
      label: "Copy page as Markdown",
      run: () =>
        void backend()
          .getPage(props.name, props.pageKind)
          .then((p) => {
            if (p) backend().writeText(p.blocks.map((b) => dtoSubtreeMarkdown(b)).join("\n"));
            pushToast("Copied page as Markdown", "success");
          }),
    },
    { label: "Export to PDF…", run: () => openPdfExport(props.name) },
    { label: "Page properties…", run: () => openPageProps(props.name, props.x, props.y) },
    // Carry a past day's unfinished tasks to today (journal days only, not today).
    ...(props.pageKind === "journal" && props.name !== journalTitle(new Date())
      ? [{ label: "Carry unfinished tasks → today", run: () => void carryDay(props.name) }]
      : []),
  ];
  return (
    <>
      <For each={items}>
        {(it) => (
          <div class="ctx-item" classList={{ danger: !!it.danger }} onClick={() => { it.run(); props.close(); }}>
            {it.label}
          </div>
        )}
      </For>
      {/* Rename is page-only. It expands into an inline input (like MakeTemplate)
          because window.prompt is a silent no-op in WebKitGTK. */}
      <Show when={pageMenuAvailability(props.pageKind).rename}>
        <RenamePage name={props.name} pageKind={props.pageKind} close={props.close} />
      </Show>
      <Show when={pageMenuAvailability(props.pageKind).delete}>
        <div class="ctx-item danger" onClick={() => { void remove(); props.close(); }}>
          {deletePageMenuLabel(props.pageKind)}
        </div>
      </Show>
    </>
  );
}

export function pageMenuAvailability(pageKind: PageKind): { rename: boolean; delete: boolean } {
  return { rename: pageKind === "page", delete: true };
}

export function deletePageMenuLabel(pageKind: PageKind): string {
  return pageKind === "journal" ? "Delete journal" : "Delete page";
}

// Inline page rename: a context-menu item that expands into a name field (mirrors
// MakeTemplate), then runs the two-phase rename transaction. window.prompt is a
// silent no-op in this WebKitGTK build, so we never use it.
function RenamePage(props: {
  name: string;
  pageKind: PageKind;
  close: () => void;
}): JSX.Element {
  const [editing, setEditing] = createSignal(false);
  const [value, setValue] = createSignal(props.name);

  const submit = async () => {
    // Snapshot everything we need BEFORE close() disposes this component — reading
    // props.* afterward warns "stale read from <Show>".
    const from = props.name;
    const kind = props.pageKind;
    const next = value().trim();
    props.close();
    if (!next || next === from) return;
    try {
      // Persist ALL unsaved edits first — the rename reads every referencing page
      // from disk to rewrite its `[[refs]]`, so a dirty edit on ANY page would be
      // read stale and lost.
      if (!(await flushAll())) {
        pushToast("Couldn't save pending edits — resolve the conflict before renaming.", "error");
        return;
      }
      await backend().renamePage(from, next);
      // Backend rewrote refs across pages via the self-write guard (no watcher
      // reload) → in-memory pages are stale; reset + reload so a stale save can't
      // revert the rename.
      refreshAfterRename();
      openPage(next, kind);
      pushToast(`Renamed to “${next}”`, "success");
    } catch (e) {
      pushToast(`Rename failed: ${String(e)}`, "error");
    }
  };

  return (
    <Show
      when={editing()}
      fallback={
        <div
          class="ctx-item"
          onClick={(e) => { e.stopPropagation(); setValue(props.name); setEditing(true); }}
        >
          Rename page…
        </div>
      }
    >
      <div class="ctx-rename-form" onClick={(e) => e.stopPropagation()}>
        <input
          class="ctx-rename-name"
          ref={(el) => queueMicrotask(() => (el.focus(), el.select()))}
          value={value()}
          onInput={(e) => setValue(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") { e.preventDefault(); void submit(); }
            else if (e.key === "Escape") { e.preventDefault(); setEditing(false); }
          }}
        />
      </div>
    </Show>
  );
}

function blockActions(id: string): { label: string; run: () => void; danger?: boolean }[] {
  const numbered = blockProperty(id, "logseq.order-list-type") === "number";
  // If this block is itself a template (`template:: name`), offer to set it as the
  // new-journal default (or clear it if it already is) — right where templates live.
  const tmplName = blockProperty(id, "template");
  const isJournalTmpl = !!tmplName && graphMeta()?.default_journal_template === tmplName;
  return [
    { label: "Open in sidebar", run: () => openBlockInSidebar(persistentBlockRef(id)) },
    { label: "Zoom into block", run: () => zoomInto(id) },
    { label: "Copy block ref", run: () => void copyBlockRef(id, (u) => `((${u}))`, "Copied block ref") },
    { label: "Copy block embed", run: () => void copyBlockRef(id, (u) => `{{embed ((${u}))}}`, "Copied block embed") },
    { label: "Copy block", run: () => { void copyOutline(blockSubtreeMarkdown(id, 0, true, copyStripCollapsed())); pushToast("Copied block", "success"); } },
    // Open the export modal for the whole selection (if this block is part of a
    // multi-selection) or just this block's subtree — preview + indent/remove opts.
    {
      label: "Copy / export as…",
      run: () => {
        const sel = selectedIds();
        openExportModal(sel.length > 1 && sel.includes(id) ? sel : [id]);
      },
    },
    ...(canConvertPipeTableToGrid(id)
      ? [{ label: "Convert to grid", run: () => { convertPipeTableToGrid(id); } }]
      : []),
    {
      label: "Cut block",
      run: () => {
        void copyOutline(blockSubtreeMarkdown(id, 0, true, copyStripCollapsed()));
        deleteBlock(id);
      },
    },
    {
      label: numbered ? "Remove numbered list" : "Numbered list",
      run: () => toggleBlockProperty(id, "logseq.order-list-type", "number"),
    },
    { label: "Collapse all", run: () => setCollapsedDeep(id, true) },
    { label: "Expand all", run: () => setCollapsedDeep(id, false) },
    ...(tmplName
      ? [
          {
            label: isJournalTmpl ? "✓ Used for new journals" : "Use for new journals",
            run: () => setJournalTemplate(isJournalTmpl ? null : tmplName),
          },
        ]
      : []),
    { label: "Delete block", run: () => deleteBlock(id), danger: true },
  ];
}
