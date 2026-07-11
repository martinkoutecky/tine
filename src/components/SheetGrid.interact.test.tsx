import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { readFileSync } from "node:fs";
import { Block, SurfaceContext } from "./Block";
import { SheetGrid } from "./SheetGrid";
import { ContextMenu } from "./ContextMenu";
import { initParser } from "../render/parse";
import {
  doc,
  blockProperty,
  blockIsGridView,
  hasSelection,
  isSelected,
  resetStore,
  selectBlock,
  setDoc,
  undo,
  type FeedPage,
  type Node as StoreNode,
} from "../store";
import { editingId, endEdit } from "../editorController";
import { installKeybindings } from "../keybindings";
import { setFocusedPaneId } from "../panes";
import {
  cellSel,
  colSeamSel,
  handleCellSelectionKey,
  resetCellSelectionForTests,
  rowSeamSel,
  setCellSel,
} from "../sheet/selection";

beforeAll(async () => {
  await initParser();
});

let disposeKeys: (() => void) | null = null;
let restoreCaretRange: (() => void) | null = null;
let restoreClipboard: (() => void) | null = null;

afterEach(() => {
  disposeKeys?.();
  disposeKeys = null;
  restoreCaretRange?.();
  restoreCaretRange = null;
  restoreClipboard?.();
  restoreClipboard = null;
  resetCellSelectionForTests();
  setFocusedPaneId("main");
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(roots: string[]): FeedPage {
  return {
    name: "Sheet",
    kind: "page",
    title: "Sheet",
    preBlock: null,
    roots,
    format: "md",
    readOnly: false,
    guide: false,
  };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function loadSheetDoc() {
  setDoc({
    byId: {
      before: node("before", "Before", null),
      grid: node("grid", "Grid parent\ntine.view:: grid\ntine.col-widths:: 0=120;1=80", null, ["r1", "r2"]),
      r1: node("r1", "", "grid", ["c1", "c2"]),
      c1: node("c1", "Alpha", "r1"),
      c2: node("c2", "Beta", "r1"),
      r2: node("r2", "", "grid", ["c3"]),
      c3: node("c3", "Gamma", "r2"),
      after: node("after", "After", null),
    },
    pages: [page(["before", "grid", "after"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

function loadEmptyGridDoc() {
  setDoc({
    byId: {
      grid: node("grid", "Blank\ntine.view:: grid", null),
    },
    pages: [page(["grid"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

function loadSheetDocWithSubgrid() {
  loadSheetDoc();
  setDoc({
    byId: {
      ...doc.byId,
      c2: node("c2", "Nested grid\ntine.view:: grid", "r1", ["nr1"]),
      nr1: node("nr1", "", "c2", ["n11"]),
      n11: node("n11", "Inner", "nr1"),
    },
    pages: [page(["before", "grid", "after"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

function loadSheetDocWithOutlineChild() {
  loadSheetDoc();
  setDoc({
    byId: {
      ...doc.byId,
      c1: node("c1", "Alpha", "r1", ["c1child"]),
      c1child: node("c1child", "Child", "c1"),
    },
    pages: [page(["before", "grid", "after"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

function loadSheetDocWithNestedHostGrid() {
  loadSheetDoc();
  setDoc({
    byId: {
      ...doc.byId,
      c1: node("c1", "Alpha", "r1", ["host"]),
      host: node("host", "Nested host\ntine.view:: grid", "c1", ["hr1"]),
      hr1: node("hr1", "", "host", ["hc1"]),
      hc1: node("hc1", "Inner", "hr1"),
    },
    pages: [page(["before", "grid", "after"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

function setup(): { root: HTMLDivElement; dispose: () => void } {
  loadSheetDoc();
  disposeKeys = installKeybindings();
  return mount(() => <Block id="grid" />);
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function cell(root: HTMLElement, row: number, col: number): HTMLElement {
  const el = root.querySelector(
    `.sheet-cell[data-sheet-grid-id="grid"][data-row="${row}"][data-col="${col}"]`
  ) as HTMLElement | null;
  if (!el) throw new Error(`missing cell ${row},${col}`);
  return el;
}

function keydown(target: EventTarget, key: string, init: Partial<KeyboardEvent> = {}): KeyboardEvent {
  const event = new KeyboardEvent("keydown", {
    key,
    code: init.code ?? (key === "Tab" ? "Tab" : key.startsWith("Arrow") ? key : ""),
    bubbles: true,
    cancelable: true,
    shiftKey: init.shiftKey ?? false,
    ctrlKey: init.ctrlKey ?? false,
    metaKey: init.metaKey ?? false,
    altKey: init.altKey ?? false,
  });
  target.dispatchEvent(event);
  return event;
}

function doubleClick(target: EventTarget, init: Partial<MouseEventInit> = {}): MouseEvent {
  const event = new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0, ...init });
  target.dispatchEvent(event);
  return event;
}

function pointer(type: string, x: number, y: number, init: Partial<MouseEventInit> = {}): Event {
  return new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: y, ...init });
}

function pointerEnter(target: EventTarget): Event {
  const event = new Event("pointerenter", { bubbles: false, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function textRange(root: globalThis.Node, needle: string, offset: number): Range {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let node: Text | null;
  while ((node = walker.nextNode() as Text | null)) {
    const idx = (node.textContent ?? "").indexOf(needle);
    if (idx !== -1) {
      const range = document.createRange();
      range.setStart(node, idx + offset);
      return range;
    }
  }
  throw new Error(`text node not found: ${needle}`);
}

function stubCaretRange(range: Range | null): () => number {
  const prev = (document as Document & { caretRangeFromPoint?: unknown }).caretRangeFromPoint;
  let calls = 0;
  (document as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null }).caretRangeFromPoint =
    () => {
      calls++;
      return range;
  };
  restoreCaretRange = () => {
    const docWithCaret = document as unknown as { caretRangeFromPoint?: unknown };
    if (prev === undefined) delete docWithCaret.caretRangeFromPoint;
    else docWithCaret.caretRangeFromPoint = prev;
  };
  return () => calls;
}

function selectedCell(root: HTMLElement): HTMLElement | null {
  return root.querySelector(".sheet-cell-selected") as HTMLElement | null;
}

function activeEditor(root: HTMLElement): HTMLTextAreaElement {
  const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement | null;
  if (!textarea) throw new Error("missing active editor");
  return textarea;
}

function installAppStyles(): HTMLStyleElement {
  const style = document.createElement("style");
  style.textContent = readFileSync("src/styles/app.css", "utf8");
  document.head.appendChild(style);
  return style;
}

function cssRule(selector: string): CSSStyleRule | null {
  for (const sheet of [...document.styleSheets]) {
    for (const rule of [...sheet.cssRules]) {
      if ("selectorText" in rule && rule.selectorText === selector) return rule as CSSStyleRule;
    }
  }
  return null;
}

function inputText(textarea: HTMLTextAreaElement, text: string): void {
  textarea.value = text;
  textarea.dispatchEvent(new Event("input", { bubbles: true }) as InputEvent);
}

function childrenSnapshot(): Record<string, string[]> {
  return Object.fromEntries(Object.entries(doc.byId).map(([id, n]) => [id, [...n.children]]));
}

function mockClipboard(): string[] {
  const writes: string[] = [];
  const desc = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: {
      writeText: vi.fn(async (text: string) => {
        writes.push(text);
      }),
    },
  });
  restoreClipboard = () => {
    if (desc) Object.defineProperty(navigator, "clipboard", desc);
    else Reflect.deleteProperty(navigator, "clipboard");
  };
  return writes;
}

function pasteText(text: string): ClipboardEvent {
  const event = new Event("paste", { bubbles: true, cancelable: true }) as ClipboardEvent;
  Object.defineProperty(event, "clipboardData", {
    value: { getData: (type: string) => (type === "text/plain" ? text : "") },
  });
  window.dispatchEvent(event);
  return event;
}

function pasteInto(target: EventTarget, text: string): ClipboardEvent {
  const event = new Event("paste", { bubbles: true, cancelable: true }) as ClipboardEvent;
  Object.defineProperty(event, "clipboardData", {
    value: { getData: (type: string) => (type === "text/plain" ? text : "") },
  });
  target.dispatchEvent(event);
  return event;
}

function clickMenuItem(label: string): void {
  const item = [...document.body.querySelectorAll(".ctx-item")].find((el) => el.textContent?.trim() === label) as
    | HTMLElement
    | undefined;
  if (!item) throw new Error(`missing menu item: ${label}`);
  item.click();
}

describe("SheetGrid interaction", () => {
  it("bounds initial work for duplicate 100k-row Grids and reuses navigation metadata", () => {
    const byId: Record<string, StoreNode> = {};
    const rowIds: string[] = [];
    let rowLengthReads = 0;
    for (let row = 0; row < 100_001; row++) {
      const rowId = `perf-row-${row}`;
      const children = new Proxy([] as string[], {
        get(target, property, receiver) {
          if (property === "length") rowLengthReads++;
          return Reflect.get(target, property, receiver);
        },
      });
      rowIds.push(rowId);
      byId[rowId] = node(rowId, "", "perf-grid", children);
    }
    byId["perf-grid"] = node("perf-grid", "Large grid\ntine.view:: grid", null, rowIds);
    setDoc({ byId, pages: [page(["perf-grid"])], feed: ["Sheet"], loaded: true });

    const readsBeforeMount = rowLengthReads;
    const { root, dispose } = mount(() => <>
      <SurfaceContext.Provider value="pane:left"><SheetGrid id="perf-grid" /></SurfaceContext.Provider>
      <SurfaceContext.Provider value="pane:right"><SheetGrid id="perf-grid" /></SurfaceContext.Provider>
    </>);
    // 200 shared discovery reads + one 200-row active-window pass per surface.
    // A per-pane dimension scan would push this to at least 800.
    expect(rowLengthReads - readsBeforeMount).toBeLessThanOrEqual(650);
    expect(root.querySelectorAll(":scope .sheet-grid > .sheet-cell")).toHaveLength(400);
    const surfaceId = "pane:left";
    setCellSel({ gridId: "perf-grid", surfaceId, row: 0, col: 0 });
    const readsAfterMount = rowLengthReads;
    for (let step = 0; step < 40; step++) {
      handleCellSelectionKey(new KeyboardEvent("keydown", { key: "ArrowDown" }));
    }
    const finalSelection = cellSel();
    expect(finalSelection?.kind).toBe("cell");
    expect(finalSelection?.kind === "cell" ? finalSelection.row : -1).toBe(20);
    expect(rowLengthReads).toBe(readsAfterMount);

    dispose();
  });

  it("enters the focused duplicate grid from outline selection with instance-scoped history", async () => {
    loadSheetDoc();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <>
      <div data-pane-id="main">
        <SurfaceContext.Provider value="main"><SheetGrid id="grid" /></SurfaceContext.Provider>
      </div>
      <div data-pane-id="pane-2">
        <SurfaceContext.Provider value="pane:pane-2"><SheetGrid id="grid" /></SurfaceContext.Provider>
      </div>
    </>);

    const leftSecond = root.querySelector<HTMLElement>(
      '.sheet-cell[data-sheet-surface-id="main"][data-row="0"][data-col="1"]'
    )!;
    leftSecond.dispatchEvent(pointer("pointerdown", 10, 10));
    setFocusedPaneId("pane-2");
    selectBlock("grid");

    keydown(window, "Enter");
    const selected = cellSel();
    expect(selected).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });
    expect(selected?.surfaceId).toBe("pane:pane-2");

    keydown(window, "Enter");
    await tick();
    const editors = root.querySelectorAll("textarea.block-editor");
    expect(editors).toHaveLength(1);
    expect(editors[0].closest('[data-sheet-surface-id="pane:pane-2"]')).not.toBeNull();
    endEdit("select-block");
    dispose();
  });

  it("enters a uniquely rendered nested grid from outline selection", async () => {
    loadSheetDocWithSubgrid();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => (
      <div data-pane-id="main">
        <SurfaceContext.Provider value="main"><SheetGrid id="grid" /></SurfaceContext.Provider>
      </div>
    ));
    const nested = root.querySelector<HTMLElement>('[data-sheet-grid-id="c2"][data-sheet-surface-id]')!;
    const surface = nested.dataset.sheetSurfaceId!;
    expect(new Set(
      [...root.querySelectorAll<HTMLElement>('[data-sheet-grid-id="c2"][data-sheet-surface-id]')]
        .map((el) => el.dataset.sheetSurfaceId)
    )).toEqual(new Set([surface]));
    expect(blockIsGridView("c2")).toBe(true);
    selectBlock("c2");
    expect(hasSelection()).toBe(true);

    keydown(window, "Enter");
    const selected = cellSel();
    expect(selected).toEqual({ kind: "cell", gridId: "c2", row: 0, col: 0 });
    expect(selected?.surfaceId).toBe(surface);

    keydown(window, "Enter");
    await tick();
    expect(editingId()).toBe("n11");
    expect(root.querySelectorAll("textarea.block-editor")).toHaveLength(1);
    endEdit("select-block");
    dispose();
  });

  it("keeps keyboard seam navigation and overtype in the selected duplicate surface", async () => {
    loadSheetDoc();
    const { root, dispose } = mount(() => <>
      <SurfaceContext.Provider value="pane:left"><SheetGrid id="grid" /></SurfaceContext.Provider>
      <SurfaceContext.Provider value="pane:right"><SheetGrid id="grid" /></SurfaceContext.Provider>
    </>);
    const right = root.querySelector<HTMLElement>(
      '.sheet-cell[data-sheet-grid-id="grid"][data-sheet-surface-id="pane:right"][data-row="0"][data-col="0"]'
    )!;

    right.dispatchEvent(pointer("pointerdown", 10, 10));
    expect(cellSel()?.surfaceId).toBe("pane:right");

    handleCellSelectionKey(new KeyboardEvent("keydown", { key: "ArrowRight" }));
    expect(cellSel()?.kind).toBe("col-seam");
    expect(cellSel()?.surfaceId).toBe("pane:right");

    handleCellSelectionKey(new KeyboardEvent("keydown", { key: "ArrowRight" }));
    expect(cellSel()?.kind).toBe("cell");
    expect(cellSel()?.surfaceId).toBe("pane:right");

    handleCellSelectionKey(new KeyboardEvent("keydown", { key: "Z" }));
    await tick();
    const editors = root.querySelectorAll("textarea.block-editor");
    expect(editors).toHaveLength(1);
    expect(editors[0].closest('[data-sheet-surface-id="pane:right"]')).not.toBeNull();
    endEdit("select-block");
    dispose();
  });

  it("keeps nested-grid Esc ascent and the next edit in the selected duplicate surface", async () => {
    loadSheetDocWithSubgrid();
    const { root, dispose } = mount(() => <>
      <SurfaceContext.Provider value="pane:left"><SheetGrid id="grid" /></SurfaceContext.Provider>
      <SurfaceContext.Provider value="pane:right"><SheetGrid id="grid" /></SurfaceContext.Provider>
    </>);
    const nestedRight = root.querySelector<HTMLElement>(
      '.sheet-cell[data-sheet-grid-id="c2"][data-sheet-surface-id="pane:right"][data-row="0"][data-col="0"]'
    )!;

    expect(nestedRight).not.toBeNull();
    setCellSel(rowSeamSel("c2", 0, 0, "sheet:pane:right:grid"));
    expect(cellSel()?.surfaceId).toBe("sheet:pane:right:grid");

    handleCellSelectionKey(new KeyboardEvent("keydown", { key: "Escape" }));
    const outer = cellSel();
    expect(outer).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 1 });
    expect(outer?.surfaceId).toBe("pane:right");

    handleCellSelectionKey(new KeyboardEvent("keydown", { key: "Enter" }));
    await tick();
    const editors = root.querySelectorAll("textarea.block-editor");
    expect(editors).toHaveLength(1);
    expect(editors[0].closest('[data-sheet-surface-id="pane:right"]')).not.toBeNull();
    endEdit("select-block");
    dispose();
  });

  it("empty grid placeholder creates the first row and cell and enters edit", async () => {
    loadEmptyGridDoc();
    const { root, dispose } = mount(() => <Block id="grid" />);

    expect(root.textContent).not.toContain("empty grid");
    const placeholder = root.querySelector(".sheet-grid-placeholder") as HTMLElement | null;
    expect(placeholder).not.toBeNull();
    expect(placeholder!.tabIndex).toBe(0);

    placeholder!.click();
    await tick();

    expect(doc.byId.grid.children).toHaveLength(1);
    const rowId = doc.byId.grid.children[0];
    expect(doc.byId[rowId].children).toHaveLength(1);
    const cellId = doc.byId[rowId].children[0];
    expect(editingId()).toBe(cellId);
    expect(activeEditor(root).value).toBe("");

    dispose();
  });

  it("top-level grid edge buttons grow columns and rows", async () => {
    const { root, dispose } = setup();
    const grid = root.querySelector('.sheet-grid[data-sheet-grid-id="grid"]') as HTMLElement | null;
    expect(grid).not.toBeNull();

    pointerEnter(grid!);

    const addCol = grid!.querySelector(":scope > .sheet-grid-add-col") as HTMLButtonElement | null;
    const addRow = grid!.querySelector(":scope > .sheet-grid-add-row") as HTMLButtonElement | null;
    expect(addCol).not.toBeNull();
    expect(addRow).not.toBeNull();
    expect(addCol!.classList.contains("sheet-grid-add-visible")).toBe(true);
    expect(addRow!.classList.contains("sheet-grid-add-visible")).toBe(true);

    addCol!.click();
    await tick();

    expect(doc.byId.r1.children).toHaveLength(3);
    expect(editingId()).toBe(doc.byId.r1.children[2]);

    const nextAddRow = grid!.querySelector(":scope > .sheet-grid-add-row") as HTMLButtonElement | null;
    expect(nextAddRow).not.toBeNull();
    nextAddRow!.click();
    await tick();

    expect(doc.byId.grid.children).toHaveLength(3);
    const rowId = doc.byId.grid.children[2];
    expect(doc.byId[rowId].children).toHaveLength(1);
    expect(editingId()).toBe(doc.byId[rowId].children[0]);

    dispose();
  });

  it("nested grids do not render edge growth buttons", () => {
    loadSheetDocWithSubgrid();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="grid" />);
    const outer = root.querySelector('.sheet-grid[data-sheet-grid-id="grid"]') as HTMLElement | null;
    const inner = root.querySelector('.sheet-grid[data-sheet-grid-id="c2"]') as HTMLElement | null;
    expect(outer).not.toBeNull();
    expect(inner).not.toBeNull();

    pointerEnter(outer!);

    expect(outer!.querySelector(":scope > .sheet-grid-add-col")).not.toBeNull();
    expect(inner!.querySelector(":scope > .sheet-grid-add-col")).toBeNull();
    expect(inner!.querySelector(":scope > .sheet-grid-add-row")).toBeNull();

    dispose();
  });

  it("single-click selects and double-click enters edit with click-point caret", async () => {
    const { root, dispose } = setup();

    stubCaretRange(null);
    cell(root, 0, 1).dispatchEvent(pointer("pointerdown", 20, 15));
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 1 });
    expect(cell(root, 0, 1).classList.contains("sheet-cell-selected")).toBe(true);
    expect(editingId()).toBeNull();

    const calls = stubCaretRange(textRange(cell(root, 0, 0), "Alpha", 2));
    cell(root, 0, 0).dispatchEvent(pointer("pointerdown", 20, 15));
    await tick();

    expect(calls()).toBe(0);
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });
    expect(editingId()).toBeNull();

    doubleClick(cell(root, 0, 0));
    await tick();

    expect(calls()).toBe(1);
    expect(editingId()).toBe("c1");
    expect(activeEditor(root).selectionStart).toBe(2);

    dispose();
  });

  it("selects first-column sticky cells visibly and walks the left ladder through them", async () => {
    const style = installAppStyles();
    const { root, dispose } = setup();
    try {
      const first = cell(root, 0, 0);
      expect(first.dataset.sheetGridId).toBe("grid");
      expect(first.dataset.row).toBe("0");
      expect(first.dataset.col).toBe("0");
      expect(first.classList.contains("sheet-sticky-left")).toBe(true);

      first.dispatchEvent(pointer("pointerdown", 20, 15));
      await tick();

      expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });
      expect(first.classList.contains("sheet-cell-selected")).toBe(true);
      expect(getComputedStyle(first).zIndex).toBe("4");
      expect(cssRule(".sheet-cell-selected.sheet-sticky-left")?.style.getPropertyValue("box-shadow")).toContain(
        "inset 0 0 0 2px"
      );

      setCellSel({ gridId: "grid", row: 0, col: 1 });
      keydown(window, "ArrowLeft");
      expect(cellSel()).toEqual(colSeamSel("grid", 1, 0));
      keydown(window, "ArrowLeft");
      expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });
      await tick();
      expect(selectedCell(root)).toBe(first);
      keydown(window, "ArrowLeft");
      expect(cellSel()).toEqual(colSeamSel("grid", 0, 0));
      keydown(window, "ArrowLeft");
      expect(cellSel()).toEqual(colSeamSel("grid", 0, 0));
    } finally {
      dispose();
      style.remove();
    }
  });

  it("keeps grid column tracks unchanged when a selected cell enters edit", async () => {
    const { root, dispose } = setup();
    const grid = root.querySelector(".sheet-grid") as HTMLElement;

    cell(root, 0, 1).dispatchEvent(pointer("pointerdown", 20, 15));
    await tick();
    const before = grid.style.gridTemplateColumns;

    doubleClick(cell(root, 0, 1));
    await tick();

    expect(editingId()).toBe("c2");
    expect(grid.style.gridTemplateColumns).toBe(before);

    dispose();
  });

  it("uses the Esc ladder from cell edit to cell selection to outline selection", async () => {
    const { root, dispose } = setup();
    stubCaretRange(textRange(cell(root, 0, 0), "Alpha", 1));
    doubleClick(cell(root, 0, 0));
    await tick();

    keydown(activeEditor(root), "Escape");
    await tick();

    expect(editingId()).toBeNull();
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });
    expect(cell(root, 0, 0).classList.contains("sheet-cell-selected")).toBe(true);

    keydown(window, "Escape");

    expect(cellSel()).toBeNull();
    expect(hasSelection()).toBe(true);
    expect(isSelected("grid")).toBe(true);

    dispose();
  });

  it("moves selection with arrows through seams, including holes, and keeps boundary seams visible", async () => {
    const { root, dispose } = setup();

    setCellSel({ gridId: "grid", row: 0, col: 0 });
    keydown(window, "ArrowRight");
    expect(cellSel()).toEqual(colSeamSel("grid", 1, 0));
    await tick();
    expect(root.querySelector(".sheet-seam-selected")).not.toBeNull();

    keydown(window, "ArrowRight");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 1 });

    keydown(window, "ArrowDown");
    expect(cellSel()).toEqual(rowSeamSel("grid", 1, 1));

    keydown(window, "ArrowDown");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 1, col: 1 });
    expect(cell(root, 1, 1).classList.contains("sheet-hole")).toBe(true);
    expect(selectedCell(root)).toBe(cell(root, 1, 1));

    setCellSel({ gridId: "grid", row: 0, col: 0 });
    keydown(window, "ArrowUp");
    expect(cellSel()).toEqual(rowSeamSel("grid", 0, 0));

    keydown(window, "ArrowUp");
    expect(cellSel()).toEqual(rowSeamSel("grid", 0, 0));

    setCellSel({ gridId: "grid", row: 1, col: 0 });
    keydown(window, "ArrowDown");
    expect(cellSel()).toEqual(rowSeamSel("grid", 2, 0));

    keydown(window, "ArrowDown");
    expect(cellSel()).toEqual(rowSeamSel("grid", 2, 0));

    dispose();
  });

  it("tabs across cells with row wrap but not past the grid edges", () => {
    const { root, dispose } = setup();

    setCellSel({ gridId: "grid", row: 0, col: 1 });
    keydown(window, "Tab");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 1, col: 0 });
    expect(selectedCell(root)).toBe(cell(root, 1, 0));

    keydown(window, "Tab", { shiftKey: true });
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 1 });

    setCellSel({ gridId: "grid", row: 0, col: 0 });
    keydown(window, "Tab", { shiftKey: true });
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });

    dispose();
  });

  it("overtype edits through the mounted editor and Enter commits without structural mutation", async () => {
    const { root, dispose } = setup();
    const beforeChildren = childrenSnapshot();
    const beforeRaws = Object.fromEntries(Object.entries(doc.byId).map(([id, n]) => [id, n.raw]));

    setCellSel({ gridId: "grid", row: 0, col: 0 });
    keydown(window, "Z");
    await tick();

    expect(editingId()).toBe("c1");
    expect(activeEditor(root).value).toBe("Z");
    expect(doc.byId.c1.raw).toBe("Z");

    keydown(activeEditor(root), "Enter");
    await tick();

    expect(editingId()).toBeNull();
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });
    expect(childrenSnapshot()).toEqual(beforeChildren);
    expect(Object.keys(doc.byId).sort()).toEqual(Object.keys(beforeRaws).sort());
    for (const [id, raw] of Object.entries(beforeRaws)) {
      if (id === "c1") expect(doc.byId[id].raw).toBe("Z");
      else expect(doc.byId[id].raw).toBe(raw);
    }

    dispose();
  });

  it("sheet-cell editing hides tine and built-in props, rejoins them on commit, and restores raw on Escape", async () => {
    loadSheetDoc();
    setDoc("byId", "c1", "raw", "Visible body\ntine.view:: grid\nid:: abc-123");
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="grid" />);

    setCellSel({ gridId: "grid", row: 0, col: 0 });
    keydown(window, "Enter");
    await tick();

    let editor = activeEditor(root);
    expect(editor.value).toBe("Visible body");
    inputText(editor, "Changed body");
    expect(doc.byId.c1.raw).toBe("Changed body\ntine.view:: grid\nid:: abc-123");

    keydown(editor, "Enter");
    await tick();
    expect(editingId()).toBeNull();
    expect(doc.byId.c1.raw).toBe("Changed body\ntine.view:: grid\nid:: abc-123");

    keydown(window, "Enter");
    await tick();
    editor = activeEditor(root);
    inputText(editor, "Scratch");
    expect(doc.byId.c1.raw).toBe("Scratch\ntine.view:: grid\nid:: abc-123");

    keydown(editor, "Escape");
    await tick();

    expect(editingId()).toBeNull();
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });
    expect(doc.byId.c1.raw).toBe("Changed body\ntine.view:: grid\nid:: abc-123");

    dispose();
  });

  it("ArrowDown from the last editor line descends into a hosted sub-grid top seam", async () => {
    loadSheetDocWithSubgrid();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="grid" />);

    setCellSel({ gridId: "grid", row: 0, col: 1 });
    keydown(window, "Enter");
    await tick();

    const editor = activeEditor(root);
    editor.setSelectionRange(editor.value.length, editor.value.length);
    keydown(editor, "ArrowDown");
    await tick();

    expect(editingId()).toBeNull();
    expect(cellSel()).toEqual(rowSeamSel("c2", 0, 0));
    const innerGrid = root.querySelector('.sheet-grid[data-sheet-grid-id="c2"]') as HTMLElement | null;
    expect(innerGrid?.querySelector(":scope > .sheet-seam-selected")).not.toBeNull();
    const outerGrid = root.querySelector('.sheet-grid[data-sheet-grid-id="grid"]') as HTMLElement | null;
    expect(outerGrid?.querySelector(":scope > .sheet-seam-selected")).toBeNull();

    dispose();
  });

  it("ArrowDown from a sheet-cell editor descends into the first outline child", async () => {
    loadSheetDocWithOutlineChild();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="grid" />);

    setCellSel({ gridId: "grid", row: 0, col: 0 });
    keydown(window, "Enter");
    await tick();

    const editor = activeEditor(root);
    editor.setSelectionRange(editor.value.length, editor.value.length);
    keydown(editor, "ArrowDown");
    await tick();

    expect(editingId()).toBe("c1child");
    expect(activeEditor(root).value).toBe("Child");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });

    keydown(activeEditor(root), "Escape");
    await tick();

    expect(editingId()).toBeNull();
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });

    dispose();
  });

  it("mousedown on a nested outline line enters sheet-owned edit mode", async () => {
    loadSheetDocWithOutlineChild();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="grid" />);

    const nestedBody = cell(root, 0, 0).querySelector(".sheet-nested-line .sheet-cell-body") as HTMLElement | null;
    expect(nestedBody).not.toBeNull();
    nestedBody!.dispatchEvent(pointer("mousedown", 20, 20));
    await tick();

    expect(editingId()).toBe("c1child");
    expect(activeEditor(root).value).toBe("Child");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });

    dispose();
  });

  it("nested host grid faces render as grids and face clicks do not edit the host line", async () => {
    loadSheetDocWithNestedHostGrid();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="grid" />);

    const innerGrid = root.querySelector('.sheet-grid[data-sheet-grid-id="host"]') as HTMLElement | null;
    expect(innerGrid).not.toBeNull();
    innerGrid!.dispatchEvent(pointer("pointerdown", 20, 20));
    innerGrid!.dispatchEvent(pointer("mousedown", 20, 20));
    await tick();

    expect(editingId()).toBeNull();

    dispose();
  });

  it("ArrowDown mid-text in a sheet-cell editor stays in edit mode", async () => {
    loadSheetDocWithSubgrid();
    setDoc("byId", "c2", "raw", "Top\nBottom\ntine.view:: grid");
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="grid" />);

    setCellSel({ gridId: "grid", row: 0, col: 1 });
    keydown(window, "Enter");
    await tick();

    const editor = activeEditor(root);
    editor.setSelectionRange(1, 1);
    keydown(editor, "ArrowDown");
    await tick();

    expect(editingId()).toBe("c2");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 1 });

    dispose();
  });

  it("multiline paste inside a cell editor inserts plain text and creates no blocks", async () => {
    loadSheetDoc();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="grid" />);

    setCellSel({ gridId: "grid", row: 0, col: 0 });
    keydown(window, "Enter");
    await tick();

    const beforeIds = Object.keys(doc.byId).sort();
    const editor = activeEditor(root);
    editor.setSelectionRange(editor.value.length, editor.value.length);
    const event = pasteInto(editor, "\nline two\twith tab");
    await tick();

    expect(event.defaultPrevented).toBe(true);
    expect(doc.byId.c1.raw).toBe("Alpha\nline two\twith tab");
    expect(Object.keys(doc.byId).sort()).toEqual(beforeIds);
    expect(editingId()).toBe("c1");

    dispose();
  });

  it("Alt+Enter on a plain sheet cell appends a child bullet and enters it", async () => {
    loadSheetDoc();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="grid" />);

    setCellSel({ gridId: "grid", row: 0, col: 0 });
    keydown(window, "Enter");
    await tick();

    keydown(activeEditor(root), "Enter", { altKey: true });
    await tick();

    const child = doc.byId.c1.children[0];
    expect(doc.byId[child].raw).toBe("");
    expect(editingId()).toBe(child);
    expect(activeEditor(root).value).toBe("");

    dispose();
  });

  it("Alt+Enter on a compact-grid cell wraps the grid then appends a bullet", async () => {
    loadSheetDocWithSubgrid();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => <Block id="grid" />);

    setCellSel({ gridId: "grid", row: 0, col: 1 });
    keydown(window, "Enter");
    await tick();

    keydown(activeEditor(root), "Enter", { altKey: true });
    await tick();

    expect(doc.byId.c2.raw).toBe("Nested grid");
    expect(doc.byId.c2.children).toHaveLength(2);
    const [host, bullet] = doc.byId.c2.children;
    expect(doc.byId[host].raw).toBe("tine.view:: grid");
    expect(doc.byId[host].children).toEqual(["nr1"]);
    expect(doc.byId[bullet].raw).toBe("");
    expect(editingId()).toBe(bullet);

    dispose();
  });

  it("the sheet-cell context menu can add and enter a child bullet", async () => {
    loadSheetDoc();
    disposeKeys = installKeybindings();
    const { root, dispose } = mount(() => (
      <>
        <Block id="grid" />
        <ContextMenu />
      </>
    ));

    cell(root, 0, 0).dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 15, clientY: 20 }));
    await tick();
    clickMenuItem("Add child bullet");
    await tick();

    const child = doc.byId.c1.children[0];
    expect(doc.byId[child].raw).toBe("");
    expect(editingId()).toBe(child);
    expect(activeEditor(root).value).toBe("");

    dispose();
  });

  it("typing on a row seam inserts a row and mounts the editor", async () => {
    const { root, dispose } = setup();
    const before = doc.byId.grid.children.length;

    setCellSel(rowSeamSel("grid", 1, 0));
    keydown(window, "N");
    await tick();
    await tick();

    expect(doc.byId.grid.children).toHaveLength(before + 1);
    const rowId = doc.byId.grid.children[1];
    const cellId = doc.byId[rowId].children[0];
    expect(editingId()).toBe(cellId);
    expect(activeEditor(root).value).toBe("N");
    expect(doc.byId[cellId].raw).toBe("N");

    dispose();
  });

  it("Backspace on a row seam deletes the row before it and one undo restores it", () => {
    const { dispose } = setup();

    setCellSel(rowSeamSel("grid", 1, 0));
    keydown(window, "Backspace");

    expect(doc.byId.grid.children).toEqual(["r2"]);
    expect(doc.byId.r1).toBeUndefined();

    undo();
    expect(doc.byId.grid.children).toEqual(["r1", "r2"]);
    expect(doc.byId.r1.children).toEqual(["c1", "c2"]);

    dispose();
  });

  it("typing on a hole materializes exactly the missing cells", async () => {
    const { root, dispose } = setup();

    setCellSel({ gridId: "grid", row: 1, col: 1 });
    keydown(window, "H");
    await tick();
    await tick();

    expect(doc.byId.r2.children).toHaveLength(2);
    const made = doc.byId.r2.children[1];
    expect(editingId()).toBe(made);
    expect(activeEditor(root).value).toBe("H");
    expect(doc.byId[made].raw).toBe("H");

    dispose();
  });

  it("typing on a column seam inserts a column and shifts col-width keys", async () => {
    const { root, dispose } = setup();

    setCellSel(colSeamSel("grid", 1, 0));
    keydown(window, "C");
    await tick();
    await tick();

    expect(doc.byId.r1.children).toHaveLength(3);
    const made = doc.byId.r1.children[1];
    expect(editingId()).toBe(made);
    expect(activeEditor(root).value).toBe("C");
    expect(blockProperty("grid", "tine.col-widths")).toBe("0=120;2=80");

    dispose();
  });

  it("outline Enter and ArrowRight enter the selected grid block", () => {
    const { dispose } = setup();

    selectBlock("grid");
    keydown(window, "Enter");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });

    setCellSel(null);
    selectBlock("grid");
    keydown(window, "ArrowRight");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "grid", row: 0, col: 0 });

    dispose();
  });

  it("renders Shift+Arrow ranges with a focused cell", async () => {
    const { root, dispose } = setup();

    setCellSel({ gridId: "grid", row: 0, col: 0 });
    keydown(window, "ArrowRight", { shiftKey: true });
    await tick();

    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 0 },
      focus: { row: 0, col: 1 },
    });
    expect(cell(root, 0, 0).classList.contains("sheet-cell-in-range")).toBe(true);
    expect(cell(root, 0, 1).classList.contains("sheet-cell-in-range")).toBe(true);
    expect(cell(root, 0, 1).classList.contains("sheet-cell-selected")).toBe(true);

    dispose();
  });

  it("drag-selects a cell range and shift-click extends from the current anchor", async () => {
    const { root, dispose } = setup();
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => cell(root, 1, 1);

    cell(root, 0, 0).dispatchEvent(pointer("pointerdown", 10, 10));
    window.dispatchEvent(pointer("pointermove", 20, 20));
    window.dispatchEvent(pointer("pointerup", 20, 20));

    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 0 },
      focus: { row: 1, col: 1 },
    });
    expect(cell(root, 1, 1).classList.contains("sheet-cell-selected")).toBe(true);

    document.elementFromPoint = prevElementFromPoint;

    setCellSel({ gridId: "grid", row: 0, col: 1 });
    cell(root, 1, 0).dispatchEvent(pointer("pointerdown", 20, 15, { shiftKey: true }));
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "grid",
      anchor: { row: 0, col: 1 },
      focus: { row: 1, col: 0 },
    });

    dispose();
  });

  it("renders selected seams as the anchored cell edge segment", async () => {
    const originalRect = HTMLElement.prototype.getBoundingClientRect;
    const originalScrollWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollWidth");
    const originalScrollHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight");
    const rect = (left: number, top: number, width: number, height: number): DOMRect => ({
      x: left,
      y: top,
      left,
      top,
      right: left + width,
      bottom: top + height,
      width,
      height,
      toJSON: () => ({}),
    } as DOMRect);
    Object.defineProperty(HTMLElement.prototype, "scrollWidth", {
      configurable: true,
      get() {
        return this.classList.contains("sheet-grid") ? 200 : 0;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
      configurable: true,
      get() {
        return this.classList.contains("sheet-grid") ? 60 : 0;
      },
    });
    HTMLElement.prototype.getBoundingClientRect = function () {
      if (this.classList.contains("sheet-grid")) return rect(0, 0, 200, 60);
      if (this.classList.contains("sheet-cell")) {
        const row = Number(this.dataset.row ?? 0);
        const col = Number(this.dataset.col ?? 0);
        return rect(col * 100, row * 30, 100, 30);
      }
      return originalRect.call(this);
    };

    const { root, dispose } = setup();
    try {
      setCellSel(colSeamSel("grid", 1, 1));
      await tick();
      let seam = root.querySelector(".sheet-seam-selected") as HTMLElement | null;
      expect(seam?.style.height).toBe("30px");
      expect(seam?.style.top).toBe("30px");

      setCellSel(colSeamSel("grid", 0, 1));
      await tick();
      seam = root.querySelector(".sheet-seam-selected") as HTMLElement | null;
      expect(seam?.style.left).toBe("0px");
      expect(seam?.style.height).toBe("30px");
      expect(seam?.style.top).toBe("30px");

      setCellSel(rowSeamSel("grid", 1, 1));
      await tick();
      seam = root.querySelector(".sheet-seam-selected") as HTMLElement | null;
      expect(seam?.style.width).toBe("100px");
      expect(seam?.style.left).toBe("100px");
    } finally {
      dispose();
      HTMLElement.prototype.getBoundingClientRect = originalRect;
      if (originalScrollWidth) Object.defineProperty(HTMLElement.prototype, "scrollWidth", originalScrollWidth);
      else delete (HTMLElement.prototype as { scrollWidth?: number }).scrollWidth;
      if (originalScrollHeight) Object.defineProperty(HTMLElement.prototype, "scrollHeight", originalScrollHeight);
      else delete (HTMLElement.prototype as { scrollHeight?: number }).scrollHeight;
    }
  });

  it("Ctrl+D fills down through the key handler", () => {
    const { dispose } = setup();

    setCellSel({ kind: "range", gridId: "grid", anchor: { row: 0, col: 0 }, focus: { row: 1, col: 0 } });
    keydown(window, "d", { ctrlKey: true });

    expect(doc.byId.c3.raw).toBe("Alpha");
    undo();
    expect(doc.byId.c3.raw).toBe("Gamma");

    dispose();
  });

  it("mod+c copies a selected range as TSV", async () => {
    const writes = mockClipboard();
    const { dispose } = setup();

    setCellSel({ kind: "range", gridId: "grid", anchor: { row: 0, col: 0 }, focus: { row: 1, col: 1 } });
    keydown(window, "c", { ctrlKey: true });
    await tick();

    expect(writes).toEqual(["Alpha\tBeta\nGamma\t"]);

    dispose();
  });

  it("select-mode paste of a structural grid copy splats into the selected grid", async () => {
    const writes = mockClipboard();
    const { dispose } = setup();

    setCellSel({ kind: "range", gridId: "grid", anchor: { row: 0, col: 0 }, focus: { row: 1, col: 1 } });
    keydown(window, "c", { ctrlKey: true });
    await tick();
    expect(writes).toEqual(["Alpha\tBeta\nGamma\t"]);

    setCellSel({ gridId: "grid", row: 1, col: 1 });
    const event = pasteText(writes[0]);
    await tick();

    expect(event.defaultPrevented).toBe(true);
    expect(doc.byId.grid.children).toHaveLength(3);
    expect(doc.byId.r2.children.map((id) => doc.byId[id].raw)).toEqual(["Gamma", "Alpha", "Beta"]);
    const newRow = doc.byId.grid.children[2];
    expect(doc.byId[newRow].children.map((id) => doc.byId[id].raw)).toEqual(["", "Gamma", ""]);
    const anchor = doc.byId.r2.children[1];
    expect(doc.byId[anchor].children.some((id) => blockIsGridView(id))).toBe(false);

    dispose();
  });

  it("edit-mode paste of a structural grid copy nests a subgrid in the edited cell", async () => {
    const writes = mockClipboard();
    const { root, dispose } = setup();

    setCellSel({ kind: "range", gridId: "grid", anchor: { row: 0, col: 0 }, focus: { row: 1, col: 1 } });
    keydown(window, "c", { ctrlKey: true });
    await tick();
    expect(writes).toEqual(["Alpha\tBeta\nGamma\t"]);

    setCellSel({ gridId: "grid", row: 1, col: 0 });
    keydown(window, "Enter");
    await tick();
    const editor = activeEditor(root);
    const event = pasteInto(editor, writes[0]);
    await tick();

    expect(event.defaultPrevented).toBe(true);
    expect(doc.byId.c3.raw).toBe("Gamma");
    const host = doc.byId.c3.children[0];
    expect(blockIsGridView(host)).toBe(true);
    expect(doc.byId[host].raw).toBe("tine.view:: grid");
    const [row0, row1] = doc.byId[host].children;
    expect(doc.byId[row0].children.map((id) => doc.byId[id].raw)).toEqual(["Alpha", "Beta"]);
    expect(doc.byId[row1].children.map((id) => doc.byId[id].raw)).toEqual(["Gamma", ""]);

    dispose();
  });

  it("paste TSV grows the grid from the selected anchor", async () => {
    const { dispose } = setup();

    setCellSel({ gridId: "grid", row: 1, col: 1 });
    const event = pasteText("P\tQ\nR\tS");
    await tick();

    expect(event.defaultPrevented).toBe(true);
    expect(doc.byId.grid.children).toHaveLength(3);
    expect(doc.byId.r2.children.map((id) => doc.byId[id].raw)).toEqual(["Gamma", "P", "Q"]);
    const newRow = doc.byId.grid.children[2];
    expect(doc.byId[newRow].children.map((id) => doc.byId[id].raw)).toEqual(["", "R", "S"]);

    dispose();
  });
});
