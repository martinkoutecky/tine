// One sheet per block (tine#874). A grid block whose own {{query}} asked for a table or
// board drew BOTH sheets under one selection key (host, surface), so keys meant for one
// acted on the other: Delete on a selected board card cleared the grid cell at its
// coordinates, card arrows then Backspace deleted a grid row, and a selected table cell
// + Delete cleared a grid cell. Where an occurrence draws the block's grid, the grid mount
// tells the block's own query so (through `Rendered`'s `OccurrenceHostContext`) and the query draws as the list;
// nothing is written, so the view properties stay. A query shown on its own (journal
// sections, raw-only rendering) draws no grid and keeps its table or board.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { QueryMacro } from "./Macro";
import { AstBody } from "../render/body";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetStore } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import type { BlockDto, RefGroup } from "../types";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { installKeybindings } from "../keybindings";
import { cellSel, resetCellSelectionForTests } from "../sheet/selection";
import { maybeEditorIn } from "../tests/editorDriver";

beforeAll(async () => { await initParser(); });

let disposeKeys: (() => void) | null = null;
const roots = new Set<() => void>();
afterEach(() => {
  for (const dispose of roots) dispose();
  roots.clear();
  disposeKeys?.();
  disposeKeys = null;
  vi.restoreAllMocks();
  resetCellSelectionForTests();
  resetSharedQueryResultsForTests();
  resetStore();
  localStorage.clear();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): HTMLDivElement {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  roots.add(dispose);
  return root;
}

const node = (id: string, raw: string, parent: string | null, children: string[] = []): StoreNode =>
  ({ id, raw, collapsed: false, parent, page: "Sheet", children });
const result = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const groups = (): RefGroup[] => [{ page: "Work", kind: "page", blocks: [result("todo-a", "TODO Write tests"), result("todo-b", "TODO Ship it")] }];

/** A host block with `hostRaw` and, for a grid, two positional rows of cells. The engine
 *  answers the query face `face` (`tine.block-view`), no grouping, two results. */
function load(hostRaw: string, face: "table" | "board" | "list", grid: boolean, opts: { liveResults?: boolean; columns?: string[] } = {}): void {
  const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["host"], format: "md", readOnly: false, guide: false };
  const cells: Record<string, StoreNode> = grid ? {
    r0: node("r0", "", "host", ["c00", "c01"]),
    c00: node("c00", "Alpha", "r0"),
    c01: node("c01", "Beta", "r0"),
    r1: node("r1", "", "host", ["c10"]),
    c10: node("c10", "Gamma", "r1"),
  } : {};
  const work: FeedPage = { ...page, name: "Work", title: "Work", roots: ["todo-a", "todo-b"] };
  const results: Record<string, StoreNode> = opts.liveResults ? {
    "todo-a": { ...node("todo-a", "TODO Write tests", null), page: "Work" },
    "todo-b": { ...node("todo-b", "TODO Ship it", null), page: "Work" },
  } : {};
  setDoc({ byId: { host: node("host", hostRaw, null, grid ? ["r0", "r1"] : []), ...cells, ...results },
    pages: opts.liveResults ? [page, work] : [page], feed: ["Sheet"], loaded: true });
  const parse = backend().parseQuery.bind(backend());
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (...args) => {
    const parsed = await parse(...args);
    return { ...parsed, view: { ...parsed.view, group_by: "", ...(opts.columns ? { columns: opts.columns } : {}) }, block_presentation: face };
  });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups()));
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
}

const press = (key: string) =>
  document.body.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
const gridOf = (root: HTMLElement) => root.querySelector('.sheet-grid[data-sheet-grid-id="host"]');
const queryBlock = (root: HTMLElement) => root.querySelector(".query-block");

/** Wait until the query has drawn its two results, in whatever face it chose. */
async function resultsDrawn(root: HTMLElement): Promise<void> {
  await vi.waitFor(() => {
    const text = queryBlock(root)?.textContent ?? "";
    expect(text).toContain("Write tests");
    expect(text).toContain("Ship it");
  });
}

/** What the grid holds: every cell's text by row. */
const gridCells = () => doc.byId.host.children.map((row) => doc.byId[row].children.map((cell) => doc.byId[cell].raw));
const GRID = [["Alpha", "Beta"], ["Gamma"]];

describe("one sheet per block: a grid host draws its own query as the list (tine#874)", () => {
  /** Mount a dual host with the board face; wait for both the grid and the results. */
  async function boardHost(): Promise<HTMLDivElement> {
    load("{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: board", "board", true);
    disposeKeys = installKeybindings();
    const root = mount(() => <Block id="host" />);
    await resultsDrawn(root);
    await vi.waitFor(() => expect(gridOf(root)).not.toBeNull());
    return root;
  }
  /** The develop path into a card selection: a background (Mod) click selects the first
   *  card without leaving the page. Absent cards (one sheet) select nothing. */
  function selectFirstCard(root: HTMLElement): void {
    const card = root.querySelector<HTMLElement>(".sheet-board-card");
    card?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ctrlKey: true }));
    if (card) expect(cellSel()).not.toBeNull();
  }

  it("board face: a card selection + Delete never clears a grid cell", async () => {
    const root = await boardHost();
    selectFirstCard(root);
    press("Delete");
    expect(gridCells()).toEqual(GRID);
  });

  it("board face: card selection arrows then Backspace never delete a grid row", async () => {
    const root = await boardHost();
    selectFirstCard(root);
    press("ArrowDown");
    press("Backspace");
    expect(gridCells()).toEqual(GRID);
  });

  it("board face: the grid and the query's list are drawn, and the file is untouched", async () => {
    const root = await boardHost();
    expect(gridOf(root)).not.toBeNull();
    expect(root.querySelector(".sheet-board")).toBeNull();
    expect(root.querySelector(".sheet-table")).toBeNull();
    expect(queryBlock(root)!.classList.contains("query-sheet-block")).toBe(false);
    expect(doc.byId.host.raw).toBe("{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: board");
  });

  it("table face: a selected table cell + Delete never clears a grid cell; the grid and a list are drawn", async () => {
    load("{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: table", "table", true);
    disposeKeys = installKeybindings();
    const root = mount(() => <Block id="host" />);
    await resultsDrawn(root);
    await vi.waitFor(() => expect(gridOf(root)).not.toBeNull());
    // The develop path: a double click selects the table's first result title (the
    // results are not loaded blocks, so it selects instead of editing), then Delete.
    const tableCell = root.querySelector<HTMLElement>('.sheet-table .sheet-cell[data-row="0"][data-col="0"]');
    tableCell?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
    if (tableCell) expect(cellSel()).not.toBeNull();
    press("Delete");
    expect(gridCells()).toEqual(GRID);
    expect(gridOf(root)).not.toBeNull();
    expect(root.querySelector(".sheet-table")).toBeNull();
    expect(root.querySelector(".sheet-board")).toBeNull();
    expect(doc.byId.host.raw).toBe("{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: table");
  });
  it("table face: a selected grid cell + Enter never changes a query result's task state", async () => {
    // The develop path: the query table registers last in the shared (host, surface)
    // slot, so a GRID cell selection's Enter ran the table's activateCell at the same
    // coordinates and cycled an unrelated result's state (TODO -> DOING plus a LOGBOOK).
    load("{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: table", "table", true, { liveResults: true, columns: ["state"] });
    disposeKeys = installKeybindings();
    const prev = (document as unknown as { caretRangeFromPoint?: unknown }).caretRangeFromPoint;
    (document as unknown as { caretRangeFromPoint: () => null }).caretRangeFromPoint = () => null;
    try {
      const root = mount(() => <Block id="host" />);
      await resultsDrawn(root);
      await vi.waitFor(() => expect(gridOf(root)).not.toBeNull());
      const gridCell = root.querySelector<HTMLElement>('.sheet-grid[data-sheet-grid-id="host"] .sheet-cell[data-row="0"][data-col="1"]')!;
      gridCell.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: 0, clientX: 20, clientY: 15 }));
      window.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, cancelable: true, button: 0, clientX: 20, clientY: 15 }));
      await vi.waitFor(() => expect(maybeEditorIn(root, "c01")).not.toBeNull());
      maybeEditorIn(root, "c01")!.keydown(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      await vi.waitFor(() => expect(cellSel()).toMatchObject({ kind: "cell", gridId: "host", row: 0, col: 1 }));
      press("Enter");
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(doc.byId["todo-a"].raw).toBe("TODO Write tests");
      expect(doc.byId["todo-b"].raw).toBe("TODO Ship it");
      expect(gridCells()).toEqual(GRID);
    } finally {
      const withCaret = document as unknown as { caretRangeFromPoint?: unknown };
      if (prev === undefined) delete withCaret.caretRangeFromPoint; else withCaret.caretRangeFromPoint = prev;
    }
  });
});

describe("one sheet per block leaves single-sheet blocks unchanged (parity)", () => {
  it("a grid alone draws its grid", async () => {
    load("Plain grid\ntine.view:: grid", "list", true);
    const root = mount(() => <Block id="host" />);
    await vi.waitFor(() => expect(gridOf(root)).not.toBeNull());
    expect(root.querySelectorAll('.sheet-grid[data-sheet-grid-id="host"] .sheet-cell').length).toBeGreaterThanOrEqual(3);
  });

  it.each(["table", "board"] as const)("a query %s alone draws its %s and no grid", async face => {
    load(`{{query (task TODO)}}\ntine.view:: ${face}`, face, false);
    const root = mount(() => <Block id="host" />);
    await resultsDrawn(root);
    await vi.waitFor(() => expect(root.querySelector(`.sheet-${face}`)).not.toBeNull());
    expect(root.querySelectorAll(`.sheet-${face}`)).toHaveLength(1);
    expect(gridOf(root)).toBeNull();
    expect(queryBlock(root)!.classList.contains("query-sheet-block")).toBe(true);
  });

  it("a grid's own list query stays the list", async () => {
    load("{{query (task TODO)}}\ntine.view:: grid", "list", true);
    const root = mount(() => <Block id="host" />);
    await resultsDrawn(root);
    expect(gridOf(root)).not.toBeNull();
    expect(root.querySelector(".sheet-board, .sheet-table")).toBeNull();
  });
});

describe("one sheet per block inside a grid cell (tine#874)", () => {
  it("a cell block that is a grid and has its own table query draws its grid and the list", async () => {
    const cellRaw = "{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: table";
    load("Outer grid\ntine.view:: grid", "table", false);
    const page: FeedPage = { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["host"], format: "md", readOnly: false, guide: false };
    setDoc({ byId: {
      host: node("host", "Outer grid\ntine.view:: grid", null, ["r0"]),
      r0: node("r0", "", "host", ["cell"]),
      cell: node("cell", cellRaw, "r0", ["k0"]),
      k0: node("k0", "", "cell", ["k00"]),
      k00: node("k00", "Inner", "k0"),
    }, pages: [page], feed: ["Sheet"], loaded: true });
    const root = mount(() => <Block id="host" />);
    await resultsDrawn(root);
    await vi.waitFor(() => expect(root.querySelector('.sheet-grid[data-sheet-grid-id="cell"]')).not.toBeNull());
    expect(root.querySelector(".sheet-table, .sheet-board")).toBeNull();
    expect(doc.byId.cell.raw).toBe(cellRaw);
  });
});

describe("a query shown without its grid keeps its sheet (tine#874, review r2 finding 1)", () => {
  it.each(["table", "board"] as const)("journal sections: a grid host's query rendered on its own keeps its %s", async face => {
    const hostRaw = `{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: ${face}`;
    load(hostRaw, face, true);
    // The call shape of `TodaySections.tsx`: the query alone, with its host block's raw text.
    const root = mount(() => <QueryMacro body="query (task TODO)" currentPage="Sheet" hideWhenEmpty
      hostRaw={{ raw: hostRaw, format: "md" }} />);
    await resultsDrawn(root);
    await vi.waitFor(() => expect(root.querySelector(`.sheet-${face}`)).not.toBeNull());
    expect(gridOf(root)).toBeNull();
    expect(doc.byId.host.raw).toBe(hostRaw);
  });

  it("raw-only rendering: a grid host's body rendered from its raw text keeps its table", async () => {
    const hostRaw = "{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: table";
    load(hostRaw, "table", true);
    // `render/body.tsx`'s query route (reached from `RefBlocks.tsx`): the body alone, even
    // under the host's own id, mounts no grid.
    const root = mount(() => <AstBody raw={hostRaw} format="md" blockId="host" />);
    await resultsDrawn(root);
    await vi.waitFor(() => expect(root.querySelector(".sheet-table")).not.toBeNull());
    expect(gridOf(root)).toBeNull();
    expect(doc.byId.host.raw).toBe(hostRaw);
  });
});
