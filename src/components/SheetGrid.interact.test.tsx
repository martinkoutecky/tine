import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { readFileSync } from "node:fs";
import { Block } from "./Block";
import { initParser } from "../render/parse";
import {
  doc,
  blockProperty,
  hasSelection,
  isSelected,
  resetStore,
  selectBlock,
  setDoc,
  undo,
  type FeedPage,
  type Node as StoreNode,
} from "../store";
import { editingId } from "../editorController";
import { installKeybindings } from "../keybindings";
import { cellSel, colSeamSel, resetCellSelectionForTests, rowSeamSel, setCellSel } from "../sheet/selection";

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

describe("SheetGrid interaction", () => {
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
