import { boardGroupField as groupFieldForToken, formulaReferenceName } from "../sheet/boardColumns";
import { reportUiFailure } from "../uiFailure";
import { For, Show, Switch, Match, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { contextMenu, closeContextMenu, zoomInto, openBlockInSidebar, openPageInSidebar, isFavorite, toggleFavorite, openPageProps, openBlockProps, openExportModal, openPdfExport, openFormulaEditor, type ContextMenuAction, type SheetCellRemoveCtx } from "../ui";
import { isMobilePlatform } from "../nativeChrome";
import { pushToast } from "../toasts";
import { graphOwner, ownedWhen, readOwned, writeOwned } from "../owned";
import { isConflicted } from "../document";
import { graphMeta, setJournalTemplate } from "../graphSession";
import { openPage, openPageTarget, openPageTargetInNewTab, openPageAtBlock, pageTargetMatchesLoaded, type PageTarget } from "../router";
import { focusedRouter, removePageTargetAcrossPanes } from "../panes";
import "../graph"; // installs the document rename's navigation refresh handler
import { backend } from "../backend";
import { carryDay } from "../carry";
import { journalTitle, appNow } from "../journal";
import { BLOCK_COLOR_NAMES, BLOCK_COLOR_SWATCH } from "../blockColors";
import { ensureBlockId, blockSubtreeMarkdown, deleteBlock, setBlockProperty, toggleBlockProperty, toggleOwnNumberedList, blockProperty, setSelectionHeading, blockWritable, setCollapsedDeep, dtoSubtreeMarkdown, flushPage, deletePage, restoreTodayJournalInFeed, reportPageLoadRefusal, selectedIds, blockPageReadOnly, pageByName, buildClipboardPayload, insertOutlineBefore, node as docNode } from "../document";
import { renameOrMergePage, renameOutcomeMessage } from "../graph";
import { openDurableBlock } from "../blockRefActions";
import { canFlatten, flatten, hierarchify } from "../sheet/restructure";
import { canConvertPipeTableToGrid, convertGridToPipeTable, convertPipeTableToGrid } from "../sheet/conversions";
import { appendSheetCellChild, deleteColumn, setBoardGroupBy } from "../sheet/mutations";
import { cellBlockId, cellForBlockId, cellOwner, cellSel, focusCell, setCellSel } from "../sheet/selection";
import { boardGroupByOptions, fieldIdsForBlocks, fieldLabel, isFieldId, type FieldId } from "../sheet/fields";
import { startEditing } from "../editorController";
import { copyStripCollapsed } from "../copySettings";
import { copyBlockOutline, writeClipboardText } from "../clipboard";
import { cutBlocks } from "../cut";
import type { PageKind } from "../types";
import { registerTransientLayer } from "../transientLayers";

// Copy a block reference/embed — but only after the block's id:: is durably on
// disk. ensureBlockId returns null if the save couldn't land (conflict/error), in
// which case we must NOT copy a ref that would dangle after a restart.
async function copyBlockRef(id: string, fmt: (uuid: string) => string, okMsg: string) {
  let uuid: string | null;
  try {
    uuid = await ensureBlockId(id);
  } catch {
    uuid = null;
  }
  if (!uuid) {
    pushToast("Couldn't save the block id — reference not copied (resolve the conflict first).", "error");
    return;
  }
  try {
    await writeClipboardText(fmt(uuid));
    pushToast(okMsg, "success");
  } catch {
    pushToast("Couldn't copy: clipboard write failed.", "error");
  }
}

function reportCopy(write: Promise<void>, okMsg: string): void {
  void write.then(() => pushToast(okMsg, "success"))
    .catch(() => pushToast("Couldn't copy: clipboard write failed.", "error"));
}

function copyBlock(id: string): void {
  const text = blockSubtreeMarkdown(id, 0, true, copyStripCollapsed());
  reportCopy(copyBlockOutline("copy", text, buildClipboardPayload([id])), "Copied block");
}

// Right-click context menu. Universal over its target: a block (full editing
// menu — colors, headings, open/copy/cut, collapse, numbered list) or a page
// reference (open / open in sidebar / new tab / copy ref). The target is
// whatever you right-clicked, so right-clicking a [[page]] acts on the page,
// not the block that contains it.
/** Viewport-clamped placement for a menu opened at (x, y). Opens DOWN from the
 *  cursor by default; opens UP (bottom anchored at the cursor) when opening down
 *  would overflow the bottom edge; clamps to the window when the menu is larger
 *  than the viewport in either axis. Pure so the flip logic is unit-testable
 *  (the DOM can't lay out in jsdom). `margin` keeps a small gap from the edge. */
export function placeContextMenu(
  x: number,
  y: number,
  w: number,
  h: number,
  vw: number,
  vh: number,
  margin = 6,
): { left: number; top: number } {
  const left = Math.max(margin, Math.min(x, vw - w - margin));
  let top = y + h > vh - margin ? y - h : y;
  if (top < margin) top = margin;
  return { left, top };
}

/** Which side a submenu opens on, once the parent menu itself has been placed.
 *  Right by default; left when the right side would leave the window; `over`
 *  (overlaying its own menu) when the viewport is too narrow for the pair, the
 *  phone case (GH #471). Pure, because jsdom cannot lay out. */
export function placeSubmenu(
  menuLeft: number,
  menuWidth: number,
  submenuWidth: number,
  vw: number,
  margin = 6,
): "right" | "left" | "over" {
  if (menuLeft + menuWidth + submenuWidth <= vw - margin) return "right";
  if (menuLeft - submenuWidth >= margin) return "left";
  return "over";
}

export function ContextMenu(): JSX.Element {
  const close = (restoreFocus = true) => {
    const current = contextMenu();
    const owner = current?.kind === "page" ? current.focusOwner : undefined;
    closeContextMenu();
    if (restoreFocus && owner) {
      queueMicrotask(() => {
        if (!contextMenu() && owner.isConnected) owner.focus();
      });
    }
  };
  let menuEl: HTMLDivElement | undefined;
  const [place, setPlace] = createSignal<{ left: number; top: number } | null>(null);
  const [submenuSide, setSubmenuSide] = createSignal<"right" | "left" | "over">("right");

  // Viewport-aware placement. The menu opens at the click point, but a tall menu
  // opened low (e.g. "Delete namespace" near the sidebar bottom, GH nit) would
  // spill past the bottom edge and clip its own items out of reach. After it
  // renders, measure it and open UPWARD when there isn't room below (anchoring its
  // bottom at the cursor), and clamp horizontally, so every item stays on-screen.
  // Applies to every menu kind. Hidden for the one frame before measurement to
  // avoid a visible jump.
  createEffect(() => {
    const cm = contextMenu();
    setPlace(null);
    if (!cm) return;
    const { x, y } = cm;
    requestAnimationFrame(() => {
      const el = menuEl;
      if (!el || contextMenu() !== cm) return;
      const r = el.getBoundingClientRect();
      const placed = placeContextMenu(x, y, r.width, r.height, window.innerWidth, window.innerHeight);
      setPlace(placed);
      // Submenus are laid out but hidden by `visibility`, so they are measurable
      // here (GH #471). One side for the whole menu: sibling submenus opening
      // opposite ways would be worse than either.
      const widest = Math.max(
        0,
        ...[...el.querySelectorAll<HTMLElement>(".ctx-submenu-menu")].map((sub) => sub.getBoundingClientRect().width),
      );
      setSubmenuSide(placeSubmenu(placed.left, r.width, widest, window.innerWidth));
      if (cm.kind === "page") {
        el.querySelector<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')?.focus();
      }
    });
  });
  createEffect(() => {
    const cm = contextMenu();
    if (!cm) return;
    const unregister = registerTransientLayer({
      id: "context-menu",
      root: () => menuEl ?? null,
      trigger: cm.kind === "page" && cm.focusOwner ? () => cm.focusOwner ?? null : undefined,
      dismiss: () => {
        const inline = menuEl?.querySelector<HTMLInputElement>(".ctx-template-name, .ctx-rename-name");
        if (inline) { inline.dispatchEvent(new Event("tine-dismiss-inline")); return true; }
        close();
        return true;
      },
    });
    onCleanup(unregister);
  });

  return (
    <Show when={contextMenu()}>
      {(m) => (
        <div class="ctx-overlay" onClick={() => close()} onContextMenu={(e) => { e.preventDefault(); close(); }}>
          <div
            ref={menuEl}
            class="ctx-menu"
            data-submenu-side={submenuSide()}
            role={m().kind === "page" ? "menu" : undefined}
            aria-label={m().kind === "page" ? "Page actions" : undefined}
            style={{
              left: `${place()?.left ?? m().x}px`,
              top: `${place()?.top ?? m().y}px`,
              visibility: place() ? "visible" : "hidden",
            }}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              // Read the live signal, not `m()`: an inline field's Enter can
              // close the menu before this bubbled keydown arrives, and the
              // disposed <Show> accessor then throws a stale read.
              if (contextMenu()?.kind !== "page" || !menuEl) return;
              handlePageMenuKeyDown(e, menuEl, () => close());
            }}
          >
            <Switch>
              <Match when={m().kind === "block"}>
                <BlockMenu id={(m() as { blockId: string }).blockId} x={m().x} y={m().y} close={close} />
              </Match>
              <Match when={m().kind === "blockref"}>
                <BlockRefMenu
                  uuid={(m() as { uuid: string }).uuid}
                  page={(m() as { page: string }).page}
                  pageKind={(m() as { pageKind: "journal" | "page" }).pageKind}
                  path={(m() as { path?: string }).path}
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
                <Show when={m()} keyed>
                  {(page) => <PageMenu
                    name={(page as { name: string }).name}
                    pageKind={(page as { pageKind: "journal" | "page" }).pageKind}
                    path={(page as { path?: string }).path}
                    fileActions={(page as { fileActions?: boolean }).fileActions ?? false}
                    x={page.x}
                    y={page.y}
                    close={close}
                  />}
                </Show>
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

function pageMenuItems(menu: HTMLElement): HTMLButtonElement[] {
  return [...menu.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
    .filter((item) => !item.disabled && item.getAttribute("aria-disabled") !== "true");
}

function handlePageMenuKeyDown(
  event: KeyboardEvent,
  menu: HTMLElement,
  close: () => void,
) {
  const target = event.target instanceof HTMLElement ? event.target : null;
  // The inline rename field owns ordinary text-editing keys. Its first Escape
  // is a separate transient rung and remounts/focuses the rename menu item.
  if (target?.closest(".ctx-rename-form")) return;
  if (event.key === "Escape") {
    event.preventDefault();
    close();
    return;
  }
  const items = pageMenuItems(menu);
  if (!items.length) return;
  const current = items.indexOf(document.activeElement as HTMLButtonElement);
  let next: number | null = null;
  if (event.key === "ArrowDown") next = current < 0 ? 0 : (current + 1) % items.length;
  else if (event.key === "ArrowUp") next = current < 0 ? items.length - 1 : (current - 1 + items.length) % items.length;
  else if (event.key === "Home") next = 0;
  else if (event.key === "End") next = items.length - 1;
  if (next == null) return;
  event.preventDefault();
  items[next].focus();
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

/** "Show children as → Outline / Grid / Table" — flips the block's `tine.view` so its
 *  child bullets render as a sheet (or back to an outline). Shared by the plain-bullet
 *  BlockMenu and the in-sheet SheetCellMenu so both entry points behave identically. */
function ShowChildrenAsSubmenu(props: { id: string; close: () => void }): JSX.Element {
  const view = () => blockProperty(props.id, "tine.view") ?? "outline";
  const setView = (next: "outline" | "grid" | "table") => {
    setBlockProperty(props.id, "tine.view", next === "outline" ? null : next);
    props.close();
  };
  const label = (name: string, active: boolean) => `${active ? "✓ " : ""}${name}`;
  return (
    <div class="ctx-item ctx-submenu">
      <span>Show children as →</span>
      <div class="ctx-submenu-menu">
        <div class="ctx-item" onClick={() => setView("outline")}>{label("Outline", view() === "outline")}</div>
        <div class="ctx-item" onClick={() => setView("grid")}>{label("Grid", view() === "grid")}</div>
        <div class="ctx-item" onClick={() => setView("table")}>{label("Table", view() === "table")}</div>
      </div>
    </div>
  );
}

function BlockMenu(props: { id: string; x: number; y: number; close: () => void }): JSX.Element {
  const hasChildren = () => (docNode(props.id)?.children.length ?? 0) > 0;
  const readOnly = () => blockPageReadOnly(props.id);
  // A heading command applies to the active selection when there is one (GH #240).
  const headingTargets = () => {
    const selected = selectedIds();
    return selected.length ? selected : [props.id];
  };
  const headingsWritable = () => headingTargets().length > 0 && headingTargets().every(blockWritable);
  return (
    <>
      <Show when={!readOnly()}>
        {/* Color row */}
        <ColorPalette id={props.id} close={props.close} />
      </Show>

      <Show when={headingsWritable()}>
        {/* Heading row */}
        <div class="ctx-row ctx-headings">
          <button class="ctx-h" title="Automatic heading" onClick={() => { setSelectionHeading(props.id, true); props.close(); }}>
            Auto
          </button>
          <For each={[1, 2, 3, 4, 5, 6]}>
            {(h) => (
              <button class="ctx-h" title={`Heading ${h}`} onClick={() => { setSelectionHeading(props.id, h); props.close(); }}>
                H{h}
              </button>
            )}
          </For>
          <button class="ctx-h" title="Remove heading" onClick={() => { setSelectionHeading(props.id, null); props.close(); }}>
            ⌫
          </button>
        </div>
      </Show>

      <Show when={!readOnly() || headingsWritable()}><div class="ctx-sep" /></Show>

      <For each={blockActions(props.id, props.x, props.y)}>
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

      {/* Turn a plain outline into a grid/table in place. Only meaningful when the
          block actually has children to lay out. */}
      <Show when={hasChildren() && !readOnly()}>
        <div class="ctx-sep" />
        <ShowChildrenAsSubmenu id={props.id} close={props.close} />
      </Show>

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
  const canDeleteRow = () => !!props.remove?.rowId && !!docNode(props.remove.rowId);
  const canDeleteColumn = () =>
    props.remove?.gridId != null && props.remove?.col != null && !!docNode(props.remove.gridId);
  const deleteRow = () => {
    const rowId = props.remove?.rowId;
    if (rowId && docNode(rowId)) deleteBlock(rowId);
    props.close();
  };
  const deleteColumnHere = () => {
    const { gridId, col } = props.remove ?? {};
    if (gridId != null && col != null) deleteColumn(gridId, col);
    props.close();
  };
  const addChild = () => {
    const active = cellSel();
    const activeCell = active && active.kind !== "row-seam" && active.kind !== "col-seam" ? focusCell(active) : null;
    const sel = activeCell && cellBlockId(activeCell) === props.id ? activeCell : cellForBlockId(props.id);
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

  return (
    <>
      <ColorPalette id={props.id} close={props.close} />
      <div class="ctx-sep" />
      <ShowChildrenAsSubmenu id={props.id} close={props.close} />
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
  return fieldIdsForBlocks(docNode(ownerId)?.children ?? []).filter(
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
  const boardGroupField = () => groupFieldForToken(props.groupBy);
  const noGrouping = () => props.rowSource === "query" && props.groupBy === "";
  const doGroupBy = (field: FieldId | "") => {
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
                  classList={{ "ctx-active": !noGrouping() && field === boardGroupField() }}
                  onClick={() => doGroupBy(field)}
                >
                  {!noGrouping() && field === boardGroupField() ? "✓ " : ""}
                  {fieldLabel(field)}
                </div>
              )}
            </For>
            <Show when={props.rowSource === "query"}>
              <div class="ctx-item" classList={{ "ctx-active": noGrouping() }} onClick={() => doGroupBy("")}>
                {noGrouping() ? "✓ " : ""}No grouping
              </div>
            </Show>
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



// Right-click menu for an INLINE block ref `((uuid))` — acts on the referenced
// (target) block: open it in the sidebar, jump to it, or copy a ref/embed. (OG's
// menu also has delete/replace, which edit the containing block's text — those are
// just a normal edit of the block here, so they're left off this menu.)
function BlockRefMenu(props: {
  uuid: string;
  page: string;
  pageKind: "journal" | "page";
  path?: string;
  close: () => void;
}): JSX.Element {
  const items = [
    {
      label: "Open in sidebar",
      run: () => openBlockInSidebar({ uuid: props.uuid, page: props.page, pageKind: props.pageKind, path: props.path }),
    },
    { label: "Go to block", run: () => openPageAtBlock({ name: props.page, pageKind: props.pageKind, block: props.uuid, path: props.path }) },
    {
      label: "Copy block ref",
      run: () => reportCopy(writeClipboardText(`((${props.uuid}))`), "Copied block ref"),
    },
    {
      label: "Copy block embed",
      run: () => reportCopy(writeClipboardText(`{{embed ((${props.uuid}))}}`), "Copied block embed"),
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
  const hasChildren = () => (docNode(props.id)?.children.length ?? 0) > 0;

  const submit = async () => {
    const title = name().trim();
    if (!title) return;
    const owner = graphOwner();
    let existing;
    try {
      existing = await readOwned(owner, backend().listTemplates());
    } catch (error) {
      if (owner()) reportUiFailure("template-read", error);
      return;
    }
    if (existing.kind === "stale") return;
    if (existing.value.some((t) => t.name.toLowerCase() === title.toLowerCase())) {
      pushToast(`A template named “${title}” already exists.`, "error");
      return;
    }
    // A permission/read-only change can land while the name read is pending.
    if (!blockWritable(props.id)) {
      reportUiFailure("template-write", "read-only");
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
          ref={(el) => el.addEventListener("tine-dismiss-inline", () => setEditing(false), { once: true })}
          placeholder="Template name"
          autofocus
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.isComposing || e.keyCode === 229) return;
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
  path?: string;
  fileActions: boolean;
  x: number;
  y: number;
  close: (restoreFocus?: boolean) => void;
}): JSX.Element {
  // A name-only caller still pins the loaded file when the menu opens.
  const openedPage = pageByName(props.name);
  const path = props.path ?? openedPage?.id;
  const target = (): PageTarget => ({ name: props.name, pageKind: props.pageKind, ...(path ? { path } : {}) });
  const fav = () => isFavorite(props.name, props.pageKind);
  const readOnly = () => {
    const page = pageByName(props.name);
    return !pageTargetMatchesLoaded(target(), page) || !!page?.readOnly;
  };
  const runFileAction = async (reveal: boolean) => {
    const owner = graphOwner();
    const name = props.name;
    const kind = props.pageKind;
    const captured = target();
    const page = pageByName(name);
    if (!pageTargetMatchesLoaded(captured, page) || page!.guide) {
      pushToast("This page target changed; reopen the page actions menu.", "error");
      return;
    }
    // A conflicted page is not flushed (its unsavable draft is what the
    // conflict is) but its file is opened as it stands: opening or revealing
    // changes nothing on disk and is the recovery path a stuck conflict needs
    // (og I1d, master 6f8531344, GH #490).
    let conflicted = isConflicted(name);
    if (!conflicted && !page!.readOnly && !(await flushPage(name))) {
      if (owner()) pushToast(`Couldn't save “${name}”; its on-disk file was not opened.`, "error");
      return;
    }
    if (!owner()) return;
    conflicted = conflicted || isConflicted(name);
    try {
      if (!pageTargetMatchesLoaded(captured, pageByName(name))) {
        pushToast("This page target changed; reopen the page actions menu.", "error");
        return;
      }
      const opened = await readOwned(owner, backend().openPageFile(name, kind, captured.path ?? page!.id, reveal));
      if (opened.kind !== "stale" && conflicted) {
        pushToast(`“${name}” has an unresolved save conflict — this is the file as it stands on disk. Your unsaved changes stay in Tine until you resolve it.`, "info");
      }
    } catch (error) {
      const message = page!.id
        ? `Couldn't ${reveal ? "show" : "open"} the page file. (${String(error)})`
        : "This page has no on-disk file yet. Type something and let Tine save it first.";
      pushToast(message, "error");
    }
  };
  const remove = async () => {
    const owner = graphOwner();
    // Snapshot props BEFORE any await/close: the menu's <Show> disposes this
    // component the instant props.close() runs, after which reading props.* warns
    // "stale read from <Show>".
    const name = props.name;
    const kind = props.pageKind;
    const captured = target();
    // Native GTK confirm — window.confirm silently returns true here, which would
    // delete the page with no prompt.
    const confirmed = await readOwned(owner, backend().confirm(`Delete "${name}"? The file moves to the graph's .tine-trash folder.`));
    if (confirmed.kind === "stale" || !confirmed.value) return;
    if (!captured.path && (pageByName(name) !== openedPage || pageByName(name)?.id !== path)) {
      pushToast("This page target changed; reopen the page actions menu.", "error");
      return;
    }
    // Route through the store (not backend directly) so it tombstones the page and
    // cancels any pending save — otherwise a just-typed, never-saved page could be
    // recreated by a queued save right after we delete it.
    // Pane routes are retired inside the durable delete, before the page leaves
    // the working set, so no pane renders a route to a purged page (GH #376).
    void writeOwned(owner, deletePage(name, kind, captured.path, () => removePageTargetAcrossPanes(captured)))
      .then((result) => {
        if (result.kind === "stale") return;
        const ok = result.value;
        if (!ok) {
          pushToast("Delete failed", "error");
          return;
        }
        // Deleted a day IN the journals feed (in place, no navigation) → the feed
        // loader's withToday didn't re-run, so restore today's empty placeholder
        // here if it was the one deleted (#17). No-op for an older day.
        if (kind === "journal") {
          const refused = restoreTodayJournalInFeed();
          if (refused) reportPageLoadRefusal(refused);
        }
        pushToast(`Deleted “${name}”`, "success");
      })
      .catch(() => { pushToast("Delete failed", "error"); });
  };
  const items: { id: string; label: string; run: () => void; danger?: boolean }[] = [
    { id: "open", label: "Open", run: () => openPageTarget(target()) },
    { id: "open-sidebar", label: "Open in sidebar", run: () => openPageInSidebar(target()) },
    { id: "open-new-tab", label: "Open in new tab", run: () => openPageTargetInNewTab(target()) },
    { id: "favorite-toggle", label: fav() ? "Remove from favorites" : "Add to favorites", run: () => toggleFavorite(props.name, props.pageKind) },
    { id: "copy-page-ref", label: "Copy page ref", run: () => reportCopy(writeClipboardText(`[[${props.name}]]`), "Copied page ref") },
    {
      id: "copy-export",
      label: "Copy / export as…",
      run: () => {
        const page = pageByName(props.name);
        if (!pageTargetMatchesLoaded(target(), page)) {
          pushToast("This page target changed; reopen the page actions menu.", "error");
          return;
        }
        // OG 1.0.0 routes the page name through the same export modal used by
        // blocks (src/main/frontend/components/page_menu.cljs:139-143). Tine's
        // modal consumes a forest root-id list, so pass this page's exact roots.
        openExportModal([...page!.roots]);
      },
    },
    {
      id: "copy-page-markdown",
      label: "Copy page as Markdown",
      run: () => {
        const owner = graphOwner();
        const request = path
          ? backend().getPageByPath(path)
          : backend().getPage(props.name, props.pageKind);
        void readOwned(owner, request).then((result) => {
          if (result.kind === "stale") return;
          const p = result.value;
          if (!p) throw new Error("Page unavailable");
          return writeClipboardText(p.blocks.map((b) => dtoSubtreeMarkdown(b)).join("\n"));
        }).then(() => { if (owner()) pushToast("Copied page as Markdown", "success"); })
          .catch(() => { if (owner()) pushToast("Couldn't copy page as Markdown.", "error"); });
      },
    },
    ...(!isMobilePlatform ? [{ id: "export-pdf", label: "Export to PDF…", run: () => openPdfExport(props.name) }] : []),
    ...(props.fileActions && !pageByName(props.name)?.guide
      ? [
          { id: "show-in-folder", label: "Show in folder", run: () => void runFileAction(true) },
          { id: "open-default-app", label: "Open with default app", run: () => void runFileAction(false) },
        ]
      : []),
    ...(!readOnly() ? [{ id: "page-properties", label: "Page properties…", run: () => openPageProps(props.name, props.x, props.y) }] : []),
    // Carry a past day's unfinished tasks to today (journal days only, not today).
    ...(!readOnly() && props.pageKind === "journal" && props.name !== journalTitle(appNow())
      ? [{ id: "carry-unfinished", label: "Carry unfinished tasks → today", run: () => void carryDay(props.name) }]
      : []),
  ];
  return (
    <>
      <For each={items}>
        {(it) => (
          <button
            type="button"
            class="ctx-item ctx-page-item"
            classList={{ danger: !!it.danger }}
            role="menuitem"
            tabIndex={-1}
            data-page-action-id={it.id}
            onClick={() => { it.run(); props.close(false); }}
          >
            {it.label}
          </button>
        )}
      </For>
      {/* Rename is page-only. It expands into an inline input (like MakeTemplate)
          because window.prompt is a silent no-op in WebKitGTK. */}
      <Show when={!readOnly() && pageMenuAvailability(props.pageKind).rename}>
        <RenamePage name={props.name} pageKind={props.pageKind} path={path} close={props.close} />
      </Show>
      <Show when={!readOnly() && pageMenuAvailability(props.pageKind).delete}>
        <button
          type="button"
          class="ctx-item ctx-page-item danger"
          role="menuitem"
          tabIndex={-1}
          data-page-action-id={props.pageKind === "journal" ? "delete-journal" : "delete-page"}
          onClick={() => { void remove(); props.close(false); }}
        >
          {deletePageMenuLabel(props.pageKind)}
        </button>
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
  path?: string;
  close: (restoreFocus?: boolean) => void;
}): JSX.Element {
  const [editing, setEditing] = createSignal(false);
  const [value, setValue] = createSignal(props.name);
  let renameItem: HTMLButtonElement | undefined;
  let renameInput: HTMLInputElement | undefined;
  const cancel = () => {
    setEditing(false);
    queueMicrotask(() => renameItem?.focus());
  };

  createEffect(() => {
    if (!editing()) return;
    const unregister = registerTransientLayer({
      id: "context-menu-page-rename",
      parentId: "context-menu",
      root: () => renameInput ?? null,
      trigger: () => renameItem ?? null,
      dismiss: () => { cancel(); return true; },
    });
    onCleanup(unregister);
  });

  const submit = async () => {
    // Snapshot everything we need BEFORE close() disposes this component — reading
    // props.* afterward warns "stale read from <Show>".
    const from = props.name;
    const kind = props.pageKind;
    const path = props.path;
    const next = value().trim();
    const root = graphMeta()?.root;
    const router = focusedRouter();
    const tabId = router.activeId();
    const intentRevision = router.routeIntentRevision();
    const live = () => graphMeta()?.root === root && router.activeId() === tabId
      && router.routeIntentRevision() === intentRevision;
    // The rename's refresh retires this owner; it hands back its successor.
    let current = graphOwner(live);
    props.close(false);
    if (!next || next === from) return;
    try {
      const target = { name: from, pageKind: kind, ...(path ? { path } : {}) };
      const renamed = await renameOrMergePage(from, next, target, (refreshed) => { current = ownedWhen(refreshed, live); });
      if (renamed === "cancelled") return;
      const message = renameOutcomeMessage(renamed, from, next);
      if (message) {
        // `uncertain` follows a reset that retired `current`; on the same graph
        // the user must still learn the rename may have happened.
        const show = renamed === "uncertain" ? graphMeta()?.root === root : current();
        if (show) pushToast(message, renamed === "unchanged" ? "info" : "error");
        return;
      }
      if (!current()) return;
      openPage(next, kind);
      pushToast(renamed === "merged" ? `Merged into “${next}”` : `Renamed to “${next}”`, "success");
    } catch (e) {
      pushToast(`Rename failed: ${String(e)}`, "error");
    }
  };

  return (
    <Show
      when={editing()}
      fallback={
        <button
          ref={renameItem}
          type="button"
          class="ctx-item ctx-page-item"
          role="menuitem"
          tabIndex={-1}
          data-page-action-id="rename-page"
          onClick={(e) => { e.stopPropagation(); setValue(props.name); setEditing(true); }}
        >
          Rename page…
        </button>
      }
    >
      <div class="ctx-rename-form" onClick={(e) => e.stopPropagation()}>
        <input
          class="ctx-rename-name"
          ref={(el) => {
            renameInput = el;
            queueMicrotask(() => (el.focus(), el.select()));
            el.addEventListener("tine-dismiss-inline", cancel, { once: true });
          }}
          value={value()}
          onInput={(e) => setValue(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.isComposing || e.keyCode === 229) return;
            if (e.key === "Enter") { e.preventDefault(); void submit(); }
            else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); cancel(); }
          }}
        />
      </div>
    </Show>
  );
}

function blockActions(id: string, x: number, y: number): { label: string; run: () => void; danger?: boolean }[] {
  const numbered = blockProperty(id, "logseq.order-list-type") === "number";
  // If this block is itself a template (`template:: name`), offer to set it as the
  // new-journal default (or clear it if it already is) — right where templates live.
  const tmplName = blockProperty(id, "template");
  const isJournalTmpl = !!tmplName && graphMeta()?.default_journal_template === tmplName;
  if (blockPageReadOnly(id)) {
    return [
      { label: "Open in sidebar", run: () => { void openDurableBlock(id, "sidebar"); } },
      { label: "Zoom into block", run: () => zoomInto(id) },
      { label: "Open in new tab", run: () => { void openDurableBlock(id, "tab"); } },
      { label: "Copy block", run: () => copyBlock(id) },
      {
        label: "Copy / export as…",
        run: () => {
          const sel = selectedIds();
          openExportModal(sel.length > 1 && sel.includes(id) ? sel : [id]);
        },
      },
    ];
  }
  return [
    { label: "Open in sidebar", run: () => { void openDurableBlock(id, "sidebar"); } },
    { label: "Zoom into block", run: () => zoomInto(id) },
    { label: "Open in new tab", run: () => { void openDurableBlock(id, "tab"); } },
    // GH #164: in the WRITABLE arm only; the read-only arm returned above.
    { label: "Properties…", run: () => openBlockProps(id, x, y) },
    // The keyboard route to "a block above this one" is Enter at offset 0, which
    // splits; a code block owns its Enter key, so a code block first on a page
    // (or first in any subtree) left the top unreachable (GH #480).
    {
      label: "Insert block above",
      run: () => {
        const inserted = insertOutlineBefore(id, [{ raw: "", children: [] }]);
        if (inserted) startEditing(inserted, 0);
      },
    },
    { label: "Copy block ref", run: () => void copyBlockRef(id, (u) => `((${u}))`, "Copied block ref") },
    { label: "Copy block embed", run: () => void copyBlockRef(id, (u) => `{{embed ((${u}))}}`, "Copied block embed") },
    { label: "Copy block", run: () => copyBlock(id) },
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
        const text = blockSubtreeMarkdown(id, 0, true, copyStripCollapsed());
        void cutBlocks([id], text, () => blockSubtreeMarkdown(id, 0, true, copyStripCollapsed()), () => deleteBlock(id))
          .catch(() => pushToast("Couldn't cut block: clipboard write failed.", "error"));
      },
    },
    {
      label: numbered ? "Remove numbered list" : "Numbered list",
      run: () => toggleOwnNumberedList(id),
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
