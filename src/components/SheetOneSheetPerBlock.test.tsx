// One sheet per block (tine#874). A live query's records sheet (its table or board) took
// its host block's id as its sheet id, so on a block that is both a grid and a query
// table or board, the grid and the records shared one selection key: keys pressed on a
// card or title changed grid cells or deleted grid rows. The records sheet now has its own
// id, `recordsSheetId(host)`. Both sheets are still drawn; each test first proves the
// selection belongs to the records sheet (or the grid), then checks the document.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block, CollapseSurfaceContext } from "./Block";
import { PageView } from "./Page";
import { QueryMacro } from "./Macro";
import { AstBody } from "../render/body";
import { buildMatrix } from "../sheet/matrix";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { hasSelection, isSelected, resetStore, undoTopTag } from "../document";
import { doc, setDoc, type FeedPage, type Node as StoreNode } from "../document/model";
import { loadSingle } from "../document/workingSet";
import type { BlockDto, PageDto, PageRead, RefGroup } from "../types";
import { blockRunResult } from "../tests/queryReadingsTestkit";
import { installKeybindings } from "../keybindings";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { resetPaneLayoutToSingle } from "../panes";
import { editingId, endEdit } from "../editorController";
import { cellSel, recordsSheetId, resetCellSelectionForTests, setCellSel } from "../sheet/selection";
import { maybeEditorIn } from "../tests/editorDriver";

// Count coordinate matrices: `matrixForGrid` is their only builder.
vi.mock("../sheet/matrix", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../sheet/matrix")>();
  return { ...actual, buildMatrix: vi.fn(actual.buildMatrix) };
});

beforeAll(async () => { await initParser(); });

let disposeKeys: (() => void) | null = null;
let restoreClipboard: (() => void) | null = null;
let restoreCaret: (() => void) | null = null;
const roots = new Set<() => void>();
afterEach(() => {
  for (const dispose of roots) dispose();
  roots.clear();
  disposeKeys?.();
  disposeKeys = null;
  restoreClipboard?.();
  restoreClipboard = null;
  restoreCaret?.();
  restoreCaret = null;
  endEdit("blur");
  vi.restoreAllMocks();
  resetCellSelectionForTests();
  resetSharedQueryResultsForTests();
  resetStore();
  resetPaneLayoutToSingle();
  resetTabsToJournals();
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

const RECORDS = recordsSheetId("host");
const node = (id: string, raw: string, parent: string | null, children: string[] = [], page = "Sheet"): StoreNode =>
  ({ id, raw, collapsed: false, parent, page, children });
const result = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const sheetPage = (roots: string[], name = "Sheet"): FeedPage =>
  ({ name, kind: "page", title: name, preBlock: null, roots, format: "md", readOnly: false, guide: false });

interface Options {
  /** Grid rows of cell texts under the host; `null` for a host that is no grid. */
  grid?: string[][] | null;
  /** Result block ids and texts; they are loaded blocks on page Work when `live`. */
  results?: [string, string][];
  live?: boolean;
  columns?: string[];
  /** Page roots around the host. */
  before?: string[];
  after?: string[];
}

const TWO_RESULTS: [string, string][] = [["todo-a", "TODO Write tests"], ["todo-b", "TODO Ship it"]];
const GRID = [["Alpha", "Beta"], ["Gamma"]];

/** Mock the query engine: face `face`, no grouping, `results` in order. */
function mockQuery(face: "table" | "board", results: [string, string][], columns?: string[]): void {
  const parse = backend().parseQuery.bind(backend());
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (...args) => {
    const parsed = await parse(...args);
    return { ...parsed, view: { ...parsed.view, group_by: "", ...(columns ? { columns } : {}) }, block_presentation: face };
  });
  const pageOf = (id: string) => doc.byId[id]?.page ?? "Work";
  const groups: RefGroup[] = [...new Set(results.map(([id]) => pageOf(id)))].map((page) =>
    ({ page, kind: "page", blocks: results.filter(([id]) => pageOf(id) === page).map(([id, raw]) => result(id, raw)) }));
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups));
  vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
  vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
}

/** Grid cells as nodes under `gridId`: row ids `<gridId>-r<i>`, cell ids `<gridId>-c<i><j>`. */
function gridNodes(gridId: string, rows: string[][]): { rowIds: string[]; byId: Record<string, StoreNode> } {
  const byId: Record<string, StoreNode> = {};
  const rowIds = rows.map((cells, r) => {
    const rowId = `${gridId}-r${r}`;
    byId[rowId] = node(rowId, "", gridId, cells.map((_, c) => `${gridId}-c${r}${c}`));
    cells.forEach((text, c) => { byId[`${gridId}-c${r}${c}`] = node(`${gridId}-c${r}${c}`, text, rowId); });
    return rowId;
  });
  return { rowIds, byId };
}

function load(hostRaw: string, face: "table" | "board", opts: Options = {}): void {
  const grid = opts.grid === undefined ? GRID : opts.grid;
  const results = opts.results ?? TWO_RESULTS;
  const cells = grid ? gridNodes("host", grid) : { rowIds: [], byId: {} };
  const around: Record<string, StoreNode> = {};
  for (const id of [...(opts.before ?? []), ...(opts.after ?? [])]) around[id] = node(id, `Text of ${id}`, null);
  const live: Record<string, StoreNode> = opts.live
    ? Object.fromEntries(results.filter(([id]) => !cells.byId[id]).map(([id, raw]) => [id, node(id, raw, null, [], "Work")]))
    : {};
  const pages = [sheetPage([...(opts.before ?? []), "host", ...(opts.after ?? [])])];
  if (opts.live && Object.keys(live).length) pages.push(sheetPage(Object.keys(live), "Work"));
  setDoc({ byId: { host: node("host", hostRaw, null, cells.rowIds), ...cells.byId, ...around, ...live },
    pages, feed: ["Sheet"], loaded: true });
  mockQuery(face, results, opts.columns);
}

const keyInit = (key: string, mods: KeyboardEventInit = {}): KeyboardEventInit => ({ key, bubbles: true, cancelable: true, ...mods });
const press = (key: string, mods: KeyboardEventInit = {}) =>
  document.body.dispatchEvent(new KeyboardEvent("keydown", keyInit(key, mods)));
const gridOf = (root: HTMLElement, id = "host") => root.querySelector(`.sheet-grid[data-sheet-grid-id="${id}"]`);
const queryBlock = (root: HTMLElement) => root.querySelector(".query-block");
const gridCells = (gridId = "host") => (doc.byId[gridId]?.children ?? []).map((row) => doc.byId[row].children.map((cell) => doc.byId[cell].raw));

async function resultsDrawn(root: HTMLElement, texts = ["Write tests", "Ship it"]): Promise<void> {
  await vi.waitFor(() => {
    const text = queryBlock(root)?.textContent ?? "";
    for (const t of texts) expect(text).toContain(t);
  });
}

async function dualHost(face: "table" | "board", opts: Options = {}): Promise<HTMLDivElement> {
  load(`{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: ${face}`, face, opts);
  disposeKeys = installKeybindings();
  const root = mount(() => <Block id="host" />);
  await resultsDrawn(root, (opts.results ?? TWO_RESULTS).slice(0, 2).map(([, raw]) => raw.replace(/^TODO /, "")));
  await vi.waitFor(() => expect(gridOf(root)).not.toBeNull());
  await vi.waitFor(() => expect(root.querySelector(`.sheet-${face}`)).not.toBeNull());
  return root;
}

/** Mod-click (background select) on the n-th board card, and prove the records sheet holds it. */
function selectCard(root: HTMLElement, n = 0, sheet = RECORDS): void {
  const card = root.querySelectorAll<HTMLElement>(".sheet-board-card")[n];
  expect(card).toBeDefined();
  card.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ctrlKey: true }));
  expect(cellSel()).toMatchObject({ kind: "cell", gridId: sheet });
}

/** Double-click an unhydrated table title (it selects instead of editing). */
function selectTitle(root: HTMLElement, row = 0): void {
  const title = root.querySelector<HTMLElement>(`.sheet-table .sheet-cell[data-sheet-grid-id="${RECORDS}"][data-row="${row}"][data-col="0"]`);
  expect(title).not.toBeNull();
  title!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
  expect(cellSel()).toMatchObject({ kind: "cell", gridId: RECORDS, row, col: 0 });
}

function gridCellEl(root: HTMLElement, row: number, col: number, gridId = "host"): HTMLElement {
  const el = root.querySelector<HTMLElement>(`.sheet-grid[data-sheet-grid-id="${gridId}"] .sheet-cell[data-sheet-grid-id="${gridId}"][data-row="${row}"][data-col="${col}"]`);
  expect(el).not.toBeNull();
  return el!;
}

function mockClipboard(): string[] {
  const writes: string[] = [];
  const desc = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: vi.fn(async (text: string) => { writes.push(text); }) },
  });
  restoreClipboard = () => {
    if (desc) Object.defineProperty(navigator, "clipboard", desc);
    else Reflect.deleteProperty(navigator, "clipboard");
  };
  return writes;
}

function pasteText(text: string): void {
  const event = new Event("paste", { bubbles: true, cancelable: true }) as ClipboardEvent;
  Object.defineProperty(event, "clipboardData", { value: { getData: (type: string) => (type === "text/plain" ? text : "") } });
  window.dispatchEvent(event);
}

function noCaretFromPoint(): void {
  const withCaret = document as unknown as { caretRangeFromPoint?: unknown };
  const prev = withCaret.caretRangeFromPoint;
  withCaret.caretRangeFromPoint = () => null;
  restoreCaret = () => { if (prev === undefined) delete withCaret.caretRangeFromPoint; else withCaret.caretRangeFromPoint = prev; };
}

/** The block properties the backend ships on a page DTO (its facets are read off the DTO). */
const propertiesOf = (raw: string): [string, string][] =>
  raw.split("\n").slice(1).map((line) => line.split(":: ") as [string, string]);

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("a records selection on a grid host never writes the grid (tine#874)", () => {
  it("board card + Delete leaves the grid and the result unchanged", async () => {
    const root = await dualHost("board");
    selectCard(root);
    press("Delete");
    expect(gridCells()).toEqual(GRID);
    expect(doc.byId.host.raw).toBe("{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: board");
  });

  it("card ArrowDown keeps the records selection, and Backspace deletes no grid row", async () => {
    const root = await dualHost("board");
    selectCard(root, 0);
    press("ArrowDown");
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: RECORDS, row: 1, col: 0 });
    press("Backspace");
    expect(gridCells()).toEqual(GRID);
  });

  it("double-click on an unhydrated table title + Delete leaves the grid unchanged", async () => {
    const root = await dualHost("table");
    selectTitle(root);
    press("Delete");
    expect(gridCells()).toEqual(GRID);
    expect(gridOf(root)).not.toBeNull();
    expect(root.querySelector(".sheet-table")).not.toBeNull();
  });

  it("Enter on a selected grid cell reopens the grid cell editor and leaves an unrelated task alone", async () => {
    // Develop: the table registered last in the shared slot, so a GRID cell selection's
    // Enter ran the table's activateCell at the same coordinates (TODO -> DOING + LOGBOOK).
    noCaretFromPoint();
    const root = await dualHost("table", { live: true, columns: ["state"] });
    const cell = gridCellEl(root, 0, 1);
    cell.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: 0, clientX: 20, clientY: 15 }));
    window.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, cancelable: true, button: 0, clientX: 20, clientY: 15 }));
    await vi.waitFor(() => expect(maybeEditorIn(root, "host-c01")).not.toBeNull());
    maybeEditorIn(root, "host-c01")!.keydown(new KeyboardEvent("keydown", keyInit("Escape")));
    await vi.waitFor(() => expect(cellSel()).toMatchObject({ kind: "cell", gridId: "host", row: 0, col: 1 }));
    press("Enter");
    await vi.waitFor(() => expect(maybeEditorIn(root, "host-c01")).not.toBeNull());
    expect(editingId()).toBe("host-c01");
    expect(doc.byId["todo-a"].raw).toBe("TODO Write tests");
    expect(doc.byId["todo-b"].raw).toBe("TODO Ship it");
    expect(gridCells()).toEqual(GRID);
  });
});

describe("records verbs on a grid host change nothing and record no history (tine#874)", () => {
  async function titleSelected(): Promise<HTMLDivElement> {
    const root = await dualHost("table", { columns: ["state"] });
    selectTitle(root);
    return root;
  }
  const unchanged = (tag: string | null) => {
    expect(gridCells()).toEqual(GRID);
    expect(undoTopTag()).toBe(tag);
  };

  it("Backspace", async () => {
    await titleSelected();
    const tag = undoTopTag();
    press("Backspace");
    unchanged(tag);
  });

  it("text paste", async () => {
    await titleSelected();
    const tag = undoTopTag();
    pasteText("Pasted");
    unchanged(tag);
  });

  it("structural paste of copied grid cells", async () => {
    const root = await titleSelected();
    const writes = mockClipboard();
    setCellSel({ kind: "range", gridId: "host", anchor: { row: 0, col: 0 }, focus: { row: 0, col: 1 } });
    press("c", { ctrlKey: true });
    await vi.waitFor(() => expect(writes.length).toBe(1));
    selectTitle(root);
    const tag = undoTopTag();
    pasteText(writes[0]);
    unchanged(tag);
  });

  it("fill down and fill right over a records range", async () => {
    await titleSelected();
    press("ArrowDown", { shiftKey: true });
    press("ArrowRight", { shiftKey: true });
    expect(cellSel()).toMatchObject({ kind: "range", gridId: RECORDS, anchor: { row: 0, col: 0 }, focus: { row: 1, col: 1 } });
    const tag = undoTopTag();
    press("d", { ctrlKey: true });
    press("r", { ctrlKey: true });
    unchanged(tag);
  });

  it("Mod+arrow (the move fallback)", async () => {
    await titleSelected();
    const tag = undoTopTag();
    press("ArrowDown", { ctrlKey: true });
    press("ArrowRight", { ctrlKey: true });
    unchanged(tag);
  });

  it("a whole-row records range + Delete", async () => {
    await titleSelected();
    press(" ", { shiftKey: true, code: "Space" });
    expect(cellSel()).toMatchObject({ kind: "range", gridId: RECORDS, anchor: { row: 0, col: 0 }, focus: { row: 0, col: 1 } });
    const tag = undoTopTag();
    press("Delete");
    unchanged(tag);
  });
});

describe("cut and copy from records read no grid cell (tine#874)", () => {
  it("a card at (0,0) + Mod+X: the clipboard write succeeds, grid (0,0) is unchanged and holds no grid text", async () => {
    const root = await dualHost("board");
    const writes = mockClipboard();
    selectCard(root);
    press("x", { ctrlKey: true });
    await vi.waitFor(() => expect(writes.length).toBe(1));
    await settle();
    expect(writes[0]).not.toContain("Alpha");
    expect(gridCells()).toEqual(GRID);
  });

  it("Mod+C copies no grid text", async () => {
    const root = await dualHost("board");
    const writes = mockClipboard();
    selectCard(root);
    press("c", { ctrlKey: true });
    await vi.waitFor(() => expect(writes.length).toBe(1));
    expect(writes[0]).not.toContain("Alpha");
  });
});

describe("pointer ranges stay on the sheet they started on (tine#874)", () => {
  function stubElementFromPoint(target: () => Element | null): void {
    const prev = document.elementFromPoint;
    document.elementFromPoint = () => target();
    const restore = restoreCaret;
    restoreCaret = () => { document.elementFromPoint = prev; restore?.(); };
  }
  function drag(from: HTMLElement): void {
    from.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: 0, clientX: 10, clientY: 10 }));
    window.dispatchEvent(new MouseEvent("pointermove", { bubbles: true, cancelable: true, button: 0, clientX: 40, clientY: 60 }));
    window.dispatchEvent(new MouseEvent("pointerup", { bubbles: true, cancelable: true, button: 0, clientX: 40, clientY: 60 }));
  }
  const titleAt = (root: HTMLElement, row: number) =>
    root.querySelector<HTMLElement>(`.sheet-table .sheet-cell[data-row="${row}"][data-col="0"]`)!;

  it("a drag from grid (0,0) over table title (1,0) stays on grid (0,0); Delete removes no row of a one-column grid", async () => {
    noCaretFromPoint();
    const oneColumn = [["Alpha"], ["Gamma"]];
    const root = await dualHost("table", { grid: oneColumn });
    stubElementFromPoint(() => titleAt(root, 1));
    drag(gridCellEl(root, 0, 0));
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: "host", row: 0, col: 0 });
    endEdit("blur");
    setCellSel({ kind: "cell", gridId: "host", row: 0, col: 0 });
    press("Delete");
    expect(doc.byId.host.children).toHaveLength(2);
    expect(gridCells()[1]).toEqual(["Gamma"]);
  });

  it("the reverse drag, from a table title over a grid cell, does not extend into the grid", async () => {
    const root = await dualHost("table");
    stubElementFromPoint(() => gridCellEl(root, 1, 0));
    drag(titleAt(root, 0));
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: RECORDS, row: 0, col: 0 });
  });

  it("a Shift-click on the other sheet starts a new selection there", async () => {
    const root = await dualHost("table");
    setCellSel({ kind: "cell", gridId: "host", row: 0, col: 0 });
    titleAt(root, 1).dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: 0, shiftKey: true }));
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: RECORDS, row: 1, col: 0 });
  });
});

describe("a records title editor on a grid host edits like records (tine#874)", () => {
  // Grid cell (0,0) is a task that is also the table's first result.
  const opts: Options = { grid: [["TODO Write tests", "Beta"], ["Gamma"]], results: [["host-c00", "TODO Write tests"], ["todo-b", "TODO Ship it"]], live: true, columns: ["state"] };
  async function editTitle(): Promise<HTMLDivElement> {
    noCaretFromPoint();
    const root = await dualHost("table", opts);
    const title = root.querySelector<HTMLElement>(`.sheet-table .sheet-cell[data-sheet-grid-id="${RECORDS}"][data-row="0"][data-col="0"]`)!;
    title.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
    await vi.waitFor(() => expect(maybeEditorIn(root.querySelector(".sheet-table")!, "host-c00")).not.toBeNull());
    return root;
  }

  it("Tab moves to the table's (0,1), opens no grid editor and leaves grid (0,1) unchanged", async () => {
    const root = await editTitle();
    maybeEditorIn(root.querySelector(".sheet-table")!, "host-c00")!.keydown(new KeyboardEvent("keydown", keyInit("Tab")));
    await vi.waitFor(() => expect(cellSel()).toMatchObject({ kind: "cell", gridId: RECORDS, row: 0, col: 1 }));
    expect(maybeEditorIn(gridOf(root)!)).toBeNull();
    expect(gridCells()).toEqual([["TODO Write tests", "Beta"], ["Gamma"]]);
  });

  it("plain Enter commits the title, restores the records selection and inserts no child", async () => {
    const root = await editTitle();
    maybeEditorIn(root.querySelector(".sheet-table")!, "host-c00")!.keydown(new KeyboardEvent("keydown", keyInit("Enter")));
    await vi.waitFor(() => expect(editingId()).toBeNull());
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: RECORDS, row: 0, col: 0 });
    expect(gridCells()).toEqual([["TODO Write tests", "Beta"], ["Gamma"]]);
    expect(doc.byId["host-c00"].children).toEqual([]);
  });
});

describe("each sheet's window and rebase follow only its own selection (tine#874)", () => {
  it("selecting table result 250 leaves a 300-row grid's render window at 200 rows", async () => {
    const rows = Array.from({ length: 300 }, (_, i) => [`Row ${i}`]);
    const results = Array.from({ length: 260 }, (_, i): [string, string] => [`res-${i}`, `TODO Result ${i}`]);
    const root = await dualHost("table", { grid: rows, results });
    const gridRowsDrawn = () => gridOf(root)!.querySelectorAll('.sheet-cell[data-sheet-grid-id="host"][data-col="0"]:not(.sheet-header-cell)').length;
    const before = gridRowsDrawn();
    expect(before).toBeLessThanOrEqual(200);
    const more = [...root.querySelectorAll<HTMLButtonElement>(".sheet-table button")].find((b) => /more/i.test(b.textContent ?? ""));
    expect(more).toBeDefined();
    more!.click();
    await vi.waitFor(() => expect(root.querySelector(`.sheet-table .sheet-cell[data-row="250"][data-col="0"]`)).not.toBeNull());
    selectTitle(root, 250);
    await settle();
    expect(gridRowsDrawn()).toBe(before);
  });

  it("a records rebase moves only the records selection: a grid selection survives the table's reorder", async () => {
    const root = await dualHost("table");
    const sortByTitle = () => root.querySelector<HTMLElement>(".sheet-table .sheet-title-header")!.click();
    const titleRow = (text: string) => [...root.querySelectorAll<HTMLElement>(`.sheet-table .sheet-cell[data-sheet-grid-id="${RECORDS}"][data-col="0"]`)]
      .find((el) => el.textContent?.includes(text))?.getAttribute("data-row");
    expect(titleRow("Write tests")).toBe("0");
    // The records selection follows its result through the reorder (a real rebase).
    selectTitle(root, 0);
    sortByTitle();
    await vi.waitFor(() => expect(titleRow("Write tests")).toBe("1"));
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: RECORDS, row: 1, col: 0 });
    // A grid selection names a grid row the table does not have. Under one shared sheet
    // id the table's rebase would have cleared it; the table's next reorder leaves it be.
    setCellSel({ kind: "cell", gridId: "host", row: 1, col: 0, rowId: "host-r1" });
    const gridSel = cellSel();
    expect(gridSel).toMatchObject({ kind: "cell", gridId: "host", row: 1, col: 0 });
    expect((gridSel as { rowId?: string }).rowId).toBe("host-r1");
    sortByTitle();
    await vi.waitFor(() => expect(titleRow("Write tests")).toBe("0"));
    await settle();
    expect(cellSel()).toEqual(gridSel);
  });
});

describe("a zero-row grid host with a table query (tine#874)", () => {
  it("shows the first-row affordance and the table; the affordance inserts row 0 and edits its first cell", async () => {
    load("{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: table", "table", { grid: [] });
    disposeKeys = installKeybindings();
    const root = mount(() => <Block id="host" />);
    await resultsDrawn(root);
    await vi.waitFor(() => expect(root.querySelector(".sheet-table")).not.toBeNull());
    const affordance = await vi.waitFor(() => {
      const el = root.querySelector<HTMLElement>('.sheet-grid[data-sheet-grid-id="host"] .sheet-grid-placeholder');
      expect(el).not.toBeNull();
      return el!;
    });
    affordance.click();
    await vi.waitFor(() => expect(doc.byId.host.children).toHaveLength(1));
    const first = doc.byId[doc.byId.host.children[0]].children[0];
    await vi.waitFor(() => expect(editingId()).toBe(first));
  });
});

describe("vertical and left flow out of a records sheet (tine#874)", () => {
  /** Page Sheet routed in the main pane: the host, then `siblings`; zoomed into the host when `zoom`. */
  async function routed(dualHostBlocks: boolean, siblings: string[], zoom = true): Promise<HTMLDivElement> {
    const hostRaw = dualHostBlocks
      ? "{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: board"
      : "{{query (task TODO)}}\ntine.view:: board";
    const leaf = (id: string, raw: string, children: PageDto["blocks"] = []) => ({ id, raw, collapsed: false, children });
    const host = { ...leaf("host", hostRaw, dualHostBlocks ? [leaf("host-r0", "", [leaf("host-c00", "Alpha")])] : []), properties: propertiesOf(hostRaw) };
    const dto: PageDto = { name: "Sheet", kind: "page", title: "Sheet", pre_block: null, format: "md",
      blocks: [host, ...siblings.map((id) => leaf(id, `Text of ${id}`))] };
    loadSingle(dto);
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto as PageRead);
    mockQuery("board", [["todo-a", "TODO Write tests"]]);
    disposeKeys = installKeybindings();
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Sheet", pageKind: "page", ...(zoom ? { block: "host" } : {}) });
    const root = mount(() => <PageView />);
    await vi.waitFor(() => expect(root.querySelector(`${zoom ? ".zoomed-block " : ""}.sheet-board-card`)).not.toBeNull());
    return root;
  }

  it("zoomed into a dual host before a sibling: last card ArrowDown keeps the card, and Backspace deletes nothing", async () => {
    const root = await routed(true, ["sibling"]);
    expect(gridOf(root)).not.toBeNull();
    expect(root.querySelector('.zoomed-block [data-block-id="sibling"]')).toBeNull();
    selectCard(root);
    press("ArrowDown");
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: RECORDS, row: 0, col: 0 });
    press("Backspace");
    expect(doc.byId.sibling?.raw).toBe("Text of sibling");
    expect(doc.byId.host).toBeDefined();
    expect(gridCells()).toEqual([["Alpha"]]);
  });

  it("a dual host that is a routed page's only root: ArrowDown keeps the card, and Backspace changes nothing", async () => {
    const root = await routed(true, [], false);
    expect(root.querySelector(".zoomed-block")).toBeNull();
    expect(gridOf(root)).not.toBeNull();
    selectCard(root);
    press("ArrowDown");
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: RECORDS, row: 0, col: 0 });
    expect(hasSelection()).toBe(false);
    press("Backspace");
    expect(doc.byId.host).toBeDefined();
    expect(gridCells()).toEqual([["Alpha"]]);
  });

  it("ArrowUp at the top row of a records sheet on a dual host keeps the selection", async () => {
    const root = await dualHost("board", { before: ["before"] });
    selectCard(root, 0);
    press("ArrowUp");
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: RECORDS, row: 0, col: 0 });
    expect(isSelected("before")).toBe(false);
  });

  it("plain host (as develop): ArrowDown from a page's only root selects the host; ArrowUp at the top selects the block before", async () => {
    load("{{query (task TODO)}}\ntine.view:: board", "board", { grid: null, results: [["todo-a", "TODO Write tests"]], before: ["before"] });
    disposeKeys = installKeybindings();
    const root = mount(() => <><Block id="before" /><Block id="host" /></>);
    await vi.waitFor(() => expect(root.querySelector(".sheet-board-card")).not.toBeNull());
    selectCard(root);
    press("ArrowDown");
    expect(cellSel()).toBeNull();
    expect(isSelected("host")).toBe(true);
    selectCard(root);
    press("ArrowUp");
    expect(cellSel()).toBeNull();
    expect(isSelected("before")).toBe(true);
  });

  it.each([["plain", false], ["dual", true]] as const)("ArrowLeft at col 0 of a records sheet on a %s host selects the host", async (_label, dual) => {
    const raw = dual ? "{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: board" : "{{query (task TODO)}}\ntine.view:: board";
    load(raw, "board", { grid: dual ? GRID : null });
    disposeKeys = installKeybindings();
    const root = mount(() => <Block id="host" />);
    await vi.waitFor(() => expect(root.querySelector(".sheet-board-card")).not.toBeNull());
    selectCard(root);
    press("ArrowLeft");
    expect(cellSel()).toBeNull();
    expect(isSelected("host")).toBe(true);
    expect(gridCells()).toEqual(dual ? GRID : []);
  });

  /** An outer grid whose cell `qcell` is a board query; `cellView` also gives the cell
   *  its own (empty) children board, which a grid cell draws beside its query. */
  async function queryInOuterCell(cellView: string): Promise<HTMLDivElement> {
    setDoc({ byId: {
      outer: node("outer", "Outer\ntine.view:: grid", null, ["or0"]),
      or0: node("or0", "", "outer", ["qcell"]),
      qcell: node("qcell", `{{query (task TODO)}}${cellView}`, "or0"),
    }, pages: [sheetPage(["outer"])], feed: ["Sheet"], loaded: true });
    mockQuery("board", TWO_RESULTS);
    disposeKeys = installKeybindings();
    const root = mount(() => <Block id="outer" />);
    await vi.waitFor(() => expect(root.querySelector(".sheet-board-card")).not.toBeNull());
    return root;
  }

  it("Escape on a records sheet inside an outer grid cell selects that outer cell", async () => {
    const root = await queryInOuterCell("");
    selectCard(root, 0, recordsSheetId("qcell"));
    press("Escape");
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: "outer", row: 0, col: 0 });
  });

  it("a grid cell that draws its own children board beside its query board: a card selection stays (the children board no longer clears it)", async () => {
    const root = await queryInOuterCell("\ntine.view:: board");
    expect(root.querySelectorAll(".sheet-board")).toHaveLength(2);
    selectCard(root, 0, recordsSheetId("qcell"));
    await settle();
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: recordsSheetId("qcell"), row: 0, col: 0 });
  });
});

describe("the dual host's own grid behaves as develop (positive control)", () => {
  it.each([["plain grid", false], ["dual host", true]] as const)("%s: Delete, cut, paste, seam insert and row delete", async (_label, dual) => {
    noCaretFromPoint();
    const writes = mockClipboard();
    if (dual) await dualHost("board");
    else {
      load("Plain grid\ntine.view:: grid", "board");
      disposeKeys = installKeybindings();
      const root = mount(() => <Block id="host" />);
      await vi.waitFor(() => expect(gridOf(root)).not.toBeNull());
    }
    setCellSel({ kind: "cell", gridId: "host", row: 0, col: 0 });
    press("Delete");
    expect(gridCells()).toEqual([["", "Beta"], ["Gamma"]]);
    setCellSel({ kind: "cell", gridId: "host", row: 0, col: 1 });
    press("x", { ctrlKey: true });
    await vi.waitFor(() => expect(gridCells()).toEqual([["", ""], ["Gamma"]]));
    expect(writes[0]).toContain("Beta");
    setCellSel({ kind: "cell", gridId: "host", row: 1, col: 0 });
    pasteText("Pasted");
    expect(gridCells()).toEqual([["", ""], ["Pasted"]]);
    setCellSel({ kind: "cell", gridId: "host", row: 1, col: 0 });
    press("ArrowDown");
    expect(cellSel()).toMatchObject({ kind: "row-seam", gridId: "host", at: 2 });
    press("Enter");
    await vi.waitFor(() => expect(doc.byId.host.children).toHaveLength(3));
    endEdit("blur");
    setCellSel({ kind: "row-seam", gridId: "host", at: 1, col: 0, anchor: { row: 1, col: 0 } });
    press("Backspace");
    expect(doc.byId.host.children).toHaveLength(2);
  });
});

describe("folding a dual host (as develop)", () => {
  const hostRaw = "{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: table";
  it("an ordinary folded dual host shows no grid and a collapsed query header; reopening shows both", async () => {
    load(hostRaw, "table");
    setDoc({ ...doc, byId: { ...doc.byId, host: { ...doc.byId.host, collapsed: true } } });
    const root = mount(() => <Block id="host" />);
    await vi.waitFor(() => expect(queryBlock(root)).not.toBeNull());
    await settle();
    expect(gridOf(root)).toBeNull();
    expect(root.querySelector(".sheet-table")).toBeNull();
    setDoc({ ...doc, byId: { ...doc.byId, host: { ...doc.byId.host, collapsed: false } } });
    await vi.waitFor(() => expect(gridOf(root)).not.toBeNull());
    await vi.waitFor(() => expect(root.querySelector(".sheet-table")).not.toBeNull());
  });

  it("a folded dual host as a zoom root shows the grid and a collapsed query header", async () => {
    const leaf = (id: string, raw: string, children: PageDto["blocks"] = [], collapsed = false) => ({ id, raw, collapsed, children });
    const dto: PageDto = { name: "Sheet", kind: "page", title: "Sheet", pre_block: null, format: "md",
      blocks: [{ ...leaf("host", hostRaw, [leaf("host-r0", "", [leaf("host-c00", "Alpha")])], true), properties: propertiesOf(hostRaw) }] };
    loadSingle(dto);
    vi.spyOn(backend(), "getPage").mockResolvedValue(dto as PageRead);
    mockQuery("table", TWO_RESULTS);
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Sheet", pageKind: "page", block: "host" });
    const root = mount(() => <PageView />);
    await vi.waitFor(() => expect(gridOf(root)).not.toBeNull());
    await vi.waitFor(() => expect(queryBlock(root)).not.toBeNull());
    await settle();
    expect(root.querySelector(".sheet-table")).toBeNull();
  });
});

describe("a records selection in a grid at the depth fallback writes no grid cell (tine#874)", () => {
  /** A chain of nested grids host → g1 → … → g<depth>; the last is a grid with a board query
   *  and `Keep me` in row 0 cell 0 (or, at depth 6, the block under a depth-5 fallback). */
  function chain(depth: 5 | 6): void {
    const byId: Record<string, StoreNode> = {};
    const gridRaw = (i: number) => `Grid ${i}\ntine.view:: grid`;
    const last = `g${depth}`;
    const ids = ["host", ...Array.from({ length: depth }, (_, i) => `g${i + 1}`)];
    ids.forEach((id, i) => {
      const next = ids[i + 1];
      const isLast = id === last;
      const raw = isLast ? "{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: board" : gridRaw(i);
      if (isLast) {
        byId[id] = node(id, raw, null, [`${id}-r0`]);
        byId[`${id}-r0`] = node(`${id}-r0`, "", id, [`${id}-c00`]);
        byId[`${id}-c00`] = node(`${id}-c00`, "Keep me", `${id}-r0`);
      } else if (depth === 6 && id === "g5") {
        // The depth-5 grid falls back to an outline; its child g6 sits at depth 6.
        byId[id] = node(id, raw, null, [next]);
      } else {
        byId[id] = node(id, raw, null, [`${id}-r0`]);
        byId[`${id}-r0`] = node(`${id}-r0`, "", id, [next]);
      }
    });
    for (const [id, n] of Object.entries(byId)) {
      for (const child of n.children) byId[child] = { ...byId[child], parent: id };
    }
    setDoc({ byId, pages: [sheetPage(["host"])], feed: ["Sheet"], loaded: true });
    mockQuery("board", TWO_RESULTS);
  }

  it.each([5, 6] as const)("depth %i: Mod-click a board card, then Delete: Keep me stays", async (depth) => {
    chain(depth);
    disposeKeys = installKeybindings();
    const root = mount(() => <Block id="host" />);
    await vi.waitFor(() => expect(root.querySelector(".sheet-board-card")).not.toBeNull());
    selectCard(root, 0, recordsSheetId(`g${depth}`));
    press("Delete");
    expect(doc.byId[`g${depth}-c00`].raw).toBe("Keep me");
  });

  it("depth 5: from a cell editor, into the fallback grid, then Right and ArrowDown resolve through its adapter, with no matrix built", async () => {
    chain(5);
    setDoc({ ...doc, byId: { ...doc.byId,
      g5: { ...doc.byId.g5, raw: "Grid 5\ntine.view:: grid", children: ["g5-r0", "g5-r1"] },
      "g5-r0": { ...doc.byId["g5-r0"], children: ["g5-c00", "g5-c01"] },
      "g5-c01": node("g5-c01", "Beside", "g5-r0"),
      "g5-r1": node("g5-r1", "", "g5", ["g5-c10", "g5-c11"]),
      "g5-c10": node("g5-c10", "Below", "g5-r1"),
      "g5-c11": node("g5-c11", "Corner", "g5-r1"),
    } });
    disposeKeys = installKeybindings();
    const root = mount(() => <Block id="host" />);
    await vi.waitFor(() => expect(root.textContent).toContain("Corner"));
    // The fallback draws no grid element; its adapter registered on the surface of the
    // grids enclosing it. The editor is the enclosing g4 cell's (block g5, "Grid 5").
    const surface = gridOf(root, "g4")!.getAttribute("data-sheet-surface-id")!;
    setCellSel({ kind: "cell", gridId: "g4", surfaceId: surface, row: 0, col: 0 });
    press("Enter");
    const editor = await vi.waitFor(() => {
      const ed = maybeEditorIn(root, "g5");
      expect(ed).not.toBeNull();
      return ed!;
    });
    editor.select(editor.value().length);
    const built = vi.mocked(buildMatrix).mock.calls.length;
    // ArrowDown at the end of the editor enters the fallback grid at its first cell.
    editor.keydown(new KeyboardEvent("keydown", keyInit("ArrowDown")));
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: "g5", surfaceId: surface, row: 0, col: 0 });
    press("ArrowRight");
    expect(cellSel()).toMatchObject({ kind: "col-seam", gridId: "g5", at: 1 });
    press("ArrowRight");
    expect(cellSel()).toMatchObject({ kind: "cell", gridId: "g5", row: 0, col: 1 });
    press("ArrowDown");
    expect(cellSel()).toMatchObject({ kind: "row-seam", gridId: "g5", at: 1 });
    expect(vi.mocked(buildMatrix).mock.calls.length - built).toBe(0);
  });
});

describe("a stored-folded grid cell whose board query is opened locally (tine#874)", () => {
  it("Mod-click a card, then Delete: Keep me stays", async () => {
    // The local fold a live reference keeps: the cell stays stored-folded (no nested
    // grid), while its query reads the occurrence's open fold and draws its board.
    const cells = gridNodes("g1", [["Keep me"]]);
    setDoc({ byId: {
      host: node("host", "Outer\ntine.view:: grid", null, ["host-r0"]),
      "host-r0": node("host-r0", "", "host", ["g1"]),
      g1: { ...node("g1", "{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: board", "host-r0", cells.rowIds), collapsed: true },
      ...cells.byId,
    }, pages: [sheetPage(["host"])], feed: ["Sheet"], loaded: true });
    mockQuery("board", TWO_RESULTS);
    disposeKeys = installKeybindings();
    const localFold = { collapsed: (id: string, stored: boolean) => id === "g1" ? false : stored, toggle: () => {}, setMany: () => {} };
    const root = mount(() => <CollapseSurfaceContext.Provider value={localFold}><Block id="host" /></CollapseSurfaceContext.Provider>);
    await vi.waitFor(() => expect(root.querySelector(".sheet-board-card")).not.toBeNull());
    expect(gridOf(root, "g1")).toBeNull();
    selectCard(root, 0, recordsSheetId("g1"));
    press("Delete");
    expect(doc.byId["g1-c00"].raw).toBe("Keep me");
  });
});

describe("display and routes step 1 leaves as develop", () => {
  it.each([
    ["inline code", "`{{query (task TODO)}}`"],
    ["a fence", "```\n{{query (task TODO)}}\n```"],
    ["an escape", "\\{{query (task TODO)}}"],
  ])("query text in %s never changes a grid cell's children table", async (_label, body) => {
    const kids = { k0: node("k0", "Kid", "cell") };
    setDoc({ byId: {
      host: node("host", "Outer\ntine.view:: grid", null, ["host-r0"]),
      "host-r0": node("host-r0", "", "host", ["cell"]),
      cell: node("cell", `${body}\ntine.view:: table`, "host-r0", ["k0"]),
      ...kids,
    }, pages: [sheetPage(["host"])], feed: ["Sheet"], loaded: true });
    mockQuery("table", TWO_RESULTS);
    const root = mount(() => <Block id="host" />);
    await vi.waitFor(() => expect(root.querySelector('.sheet-table[data-sheet-grid-id="cell"]')).not.toBeNull());
    expect(root.querySelector('.sheet-table[data-sheet-grid-id="cell"]')!.textContent).toContain("Kid");
  });

  it("a real list query in a grid cell keeps the cell's children table", async () => {
    setDoc({ byId: {
      host: node("host", "Outer\ntine.view:: grid", null, ["host-r0"]),
      "host-r0": node("host-r0", "", "host", ["cell"]),
      cell: node("cell", "{{query (task TODO)}}\ntine.view:: table", "host-r0", ["k0"]),
      k0: node("k0", "Kid", "cell"),
    }, pages: [sheetPage(["host"])], feed: ["Sheet"], loaded: true });
    const parse = backend().parseQuery.bind(backend());
    vi.spyOn(backend(), "parseQuery").mockImplementation(async (...args) => ({ ...(await parse(...args)), block_presentation: "list" }));
    vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([{ page: "Work", kind: "page", blocks: [result("todo-a", "TODO Write tests")] }]));
    vi.spyOn(backend(), "queryOgExpressible").mockResolvedValue(true);
    vi.spyOn(backend(), "printQuery").mockResolvedValue("(task TODO)");
    const root = mount(() => <Block id="host" />);
    await vi.waitFor(() => expect(root.querySelector('.sheet-table[data-sheet-grid-id="cell"]')).not.toBeNull());
    expect(root.querySelector('.sheet-table[data-sheet-grid-id="cell"]')!.textContent).toContain("Kid");
  });

  it.each(["table", "board"] as const)("journal sections: a grid host's query shown on its own keeps its %s", async (face) => {
    const hostRaw = `{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: ${face}`;
    load(hostRaw, face);
    // The call shape of `TodaySections.tsx`: the query alone, with its host block's raw text.
    const root = mount(() => <QueryMacro body="query (task TODO)" currentPage="Sheet" hideWhenEmpty hostRaw={{ raw: hostRaw, format: "md" }} />);
    await resultsDrawn(root);
    await vi.waitFor(() => expect(root.querySelector(`.sheet-${face}`)).not.toBeNull());
    expect(gridOf(root)).toBeNull();
  });

  it("raw-only route (`RefBlocks` mounts the body with no block id): the body keeps its table", async () => {
    const hostRaw = "{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: table";
    load(hostRaw, "table");
    const root = mount(() => <AstBody raw={hostRaw} format="md" />);
    await resultsDrawn(root);
    await vi.waitFor(() => expect(root.querySelector(".sheet-table")).not.toBeNull());
    expect(gridOf(root)).toBeNull();
  });

  it("a live body under the host's id (`AstBody blockId=\"host\"`) keeps its table, under the records id", async () => {
    const hostRaw = "{{query (task TODO)}}\ntine.view:: grid\ntine.block-view:: table";
    load(hostRaw, "table");
    const root = mount(() => <AstBody raw={hostRaw} format="md" blockId="host" />);
    await resultsDrawn(root);
    await vi.waitFor(() => expect(root.querySelector(`.sheet-table[data-sheet-grid-id="${RECORDS}"]`)).not.toBeNull());
    expect(gridOf(root)).toBeNull();
  });
});
