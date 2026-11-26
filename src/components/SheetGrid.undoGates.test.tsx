// GH #874: the cases where Undo of a sheet-cell edit keeps its earlier result, so that the
// grid's "resume in the cell" rule only answers for a grid occurrence that is mounted.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { createSignal, type JSX } from "solid-js";
import { Block, SurfaceContext } from "./Block";
import { SheetBoard } from "./SheetBoard";
import { SheetTable } from "./SheetTable";
import { initParser } from "../render/parse";
import { loadFeed, node, resetStore, setRaw, toggleUndoRedoMode, undo } from "../document";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";
import { captureHistoryEditorContext, editingId, editingOwner, editingSurface, endEdit, startEditing } from "../editorController";
import {
  cellOwner, cellSel, cellSurfaceKey, handleCellSelectionKey, resetCellSelectionForTests, setCellSel, startCellEditing,
  type CellSel,
} from "../sheet/selection";
import { editorsIn } from "../tests/editorDriver";
import type { PageDto, RefGroup } from "../types";

const id = "18bb3c4f-4310-4877-97fb-6a45b57fc3e7";
const address = { gridId: "grid", surfaceId: "main", row: 0, col: 0 };

function gridPage(raw = "Alpha"): PageDto {
  return { name: "Sheet", title: "Sheet", kind: "page", pre_block: null, format: "md",
    blocks: [{ id: "grid", raw: "Grid\ntine.view:: grid", properties: [["tine.view", "grid"]], collapsed: false, children: [
      { id: "row", raw: "", collapsed: false, children: [
        { id, raw: `${raw}\nid:: ${id}`, properties: [["id", id]], collapsed: false, children: [] },
      ] },
    ] }] };
}

beforeAll(async () => {
  await initParser();
});

const mounted = new Set<() => void>();
function mount(surface: string, content: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.append(root);
  const unmount = render(() => <SurfaceContext.Provider value={surface}>{content()}</SurfaceContext.Provider>, root);
  const dispose = () => { mounted.delete(dispose); unmount(); root.remove(); };
  mounted.add(dispose);
  return { root, dispose };
}

/** Starts editing the grid's only cell, as a double-click on it does. */
function editGridCell(): void {
  setCellSel(address);
  startEditing(id, 1, cellOwner(address));
  expect(captureHistoryEditorContext()?.surface).toBe(cellSurfaceKey("grid", "main"));
}

afterEach(() => {
  for (const dispose of mounted) dispose();
  endEdit("page-navigation");
  resetCellSelectionForTests();
  resetStore();
});

describe("Undo of a sheet-cell edit outside the grid rule keeps its earlier result", () => {
  it("a reload of a cell edit leaves an ordinary copy of the block in another pane closed", () => {
    loadFeed([gridPage()]);
    const grid = mount("main", () => <Block id="grid" />);
    const copy = mount("pane:other", () => <Block id={id} />);
    editGridCell();
    expect(editorsIn(grid.root)).toHaveLength(1);

    loadFeed([gridPage("External")], { endEdit: false });
    expect(node(id)?.raw).toContain("External");
    expect(editingId()).toBe(id);
    expect(editingOwner()).toBeNull();
    expect(editingSurface()).toBe(cellSurfaceKey("grid", "main"));
    expect(editorsIn(copy.root)).toHaveLength(0);
    expect(editorsIn(grid.root)).toHaveLength(0);
  });

  it("Undo of a board card edit after a regroup restores the card's surface with no selection", () => {
    const input = gridPage();
    input.blocks = [{ id: "board", raw: "Board", collapsed: false, children: [
      { id, raw: "TODO [#A] Alpha", marker: "TODO", priority: "A", collapsed: false, children: [] },
    ] }];
    loadFeed([input]);
    const [group, setGroup] = createSignal("state");
    const board = mount("main", () => <SheetBoard ownerId="board" rowSource="children" groupBy={group()} />);
    const card = board.root.querySelector<HTMLElement>(`.sheet-board-card[data-block-id="${id}"]`)!;
    card.click();
    const selected = cellSel();
    expect(selected?.kind).toBe("cell");
    expect(startCellEditing(selected as CellSel)).toBe(true);
    expect(editorsIn(board.root)).toHaveLength(1);
    setRaw(id, "TODO [#A] Edited");
    endEdit("page-navigation");
    setGroup("priority");

    expect(undo()).toBe(true);
    expect(node(id).raw).toBe("TODO [#A] Alpha");
    expect(editingOwner()).toBeNull();
    expect(editingSurface()).toBe(cellSurfaceKey("board", "main"));
    expect(cellSel()).toBeNull();
  });

  // A grid host whose query draws a board or table: Macro mounts that view with the host's
  // own id as its owner, so both views share one host and surface.
  async function dualHost(view: "board" | "table") {
    const input = gridPage();
    const gridHost = input.blocks[0];
    gridHost.raw = `Grid\ntine.view:: grid\ntine.block-view:: ${view}`;
    gridHost.children = [0, 1].map((r) => ({ id: `r${r}`, raw: "", collapsed: false, children: [0, 1, 2].map((c) => (
      r === 0 && c === 0 ? gridHost.children[0].children[0] : { id: `g${r}${c}`, raw: `G${r}${c}`, collapsed: false, children: [] }
    )) }));
    input.blocks.push({ id: "task", raw: "TODO [#A] Task", marker: "TODO", priority: "A", collapsed: false, children: [] });
    loadFeed([input]);
    const results: RefGroup[] = [{ page: "Sheet", kind: "page", blocks: [{ id: "task", raw: node("task").raw, collapsed: false, children: [] }] }];
    const groups = adoptResults(results, captureResultsTicket()) ?? results;
    const [group, setGroup] = createSignal("state");
    const grid = mount("main", () => <Block id="grid" />);
    await new Promise((resolve) => setTimeout(resolve, 0));
    // The query's results arrive after the grid has mounted, so its view registers last.
    const sheet = mount("main", () => view === "board"
      ? <SheetBoard ownerId="grid" rowSource="query" groupBy={group()} groups={groups} />
      : <SheetTable ownerId="grid" rowSource="query" groups={groups} />);
    const gridText = () => [id, "g01", "g02", "g10", "g11", "g12"].map((b) => node(b).raw.split("\n")[0]);
    return { grid, sheet, setGroup, gridText, before: gridText() };
  }

  it.each(["board", "table"] as const)("Undo of a %s cell edit under a grid host leaves the grid cells alone", async (view) => {
    const { sheet, setGroup, gridText, before } = await dualHost(view);
    if (view === "board") sheet.root.querySelector<HTMLElement>('.sheet-board-card[data-block-id="task"]')!.click();
    else setCellSel({ gridId: "grid", surfaceId: "main", rowId: "page\0Sheet\0task", columnId: "title", row: 0, col: 0 });
    const selected = cellSel();
    expect(selected?.kind).toBe("cell");
    expect(startCellEditing(selected as CellSel)).toBe(true);
    expect(editingId()).toBe("task");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(editorsIn(sheet.root)).toHaveLength(1);
    setRaw("task", "TODO [#A] Edited");
    endEdit("page-navigation");
    if (view === "board") setGroup("priority");
    expect(toggleUndoRedoMode()).toBe("Page only");
    try {
      expect(undo()).toBe(true);
      expect(node("task").raw).toBe("TODO [#A] Task");
      handleCellSelectionKey(new KeyboardEvent("keydown", { key: "Delete" }));
      expect(gridText()).toEqual(before);
      expect(cellSel()).toBeNull();
      expect(editingOwner()).toBeNull();
      expect(editingSurface()).toBe(cellSurfaceKey("grid", "main"));
    } finally {
      toggleUndoRedoMode();
    }
  });

  it("Undo of a grid edit after its grid unmounts leaves no selection, though a board under that host is mounted", async () => {
    const { grid, gridText } = await dualHost("board");
    editGridCell();
    setRaw(id, `Edited\nid:: ${id}`);
    endEdit("page-navigation");
    const edited = gridText();
    grid.dispose();
    expect(undo()).toBe(true);
    expect(node(id).raw).toContain("Alpha");
    expect(cellSel()).toBeNull();
    expect(editingOwner()).toBeNull();
    expect(editingSurface()).toBe(cellSurfaceKey("grid", "main"));
    handleCellSelectionKey(new KeyboardEvent("keydown", { key: "Delete" }));
    expect(gridText().slice(1)).toEqual(edited.slice(1));
  });

  it("Undo of a grid edit from the zoomed view in page-only mode leaves no hidden selection for Delete", () => {
    loadFeed([gridPage()]);
    const grid = mount("main", () => <Block id="grid" />);
    editGridCell();
    setRaw(id, `Edited\nid:: ${id}`);
    endEdit("page-navigation");
    grid.dispose();
    const zoom = mount("main", () => <Block id={id} />);
    startEditing(id, 1, null, "main");
    expect(toggleUndoRedoMode()).toBe("Page only");
    try {
      expect(undo()).toBe(true);
      expect(node(id).raw).toContain("Alpha");
      expect(editingOwner()).toBeNull();
      expect(editingSurface()).toBe(cellSurfaceKey("grid", "main"));
      expect(cellSel()).toBeNull();
      expect(editorsIn(zoom.root)).toHaveLength(0);
      handleCellSelectionKey(new KeyboardEvent("keydown", { key: "Delete" }));
      expect(node(id).raw).toContain("Alpha");
    } finally {
      toggleUndoRedoMode();
    }
  });
});
