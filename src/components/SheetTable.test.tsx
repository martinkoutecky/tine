import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import { createSignal, type JSX } from "solid-js";
import { Block, SurfaceContext } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { __sheetTableTestHooks, SheetTable } from "./SheetTable";
import { DatePicker } from "./DatePicker";
import { initParser } from "../render/parse";
import { blockProperty, doc, pageByName, readPageProperty, redo, resetStore, setDoc, setRaw, undo, type FeedPage, type Node as StoreNode } from "../store";
import { setWorkflow } from "../ui";
import {
  cellSel,
  cellForBlockId,
  handleCellSelectionKey,
  resetCellSelectionForTests,
  setCellRangeSelection,
  setCellSel,
  startCellEditing,
} from "../sheet/selection";
import { editingId, editingOwner } from "../editorController";
import type { RefGroup } from "../types";
import { installKeybindings } from "../keybindings";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  __sheetTableTestHooks.onIndexRow = undefined;
  resetCellSelectionForTests();
  resetStore();
  setWorkflow("todo");
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function page(roots: string[], preBlock: string | null = null): FeedPage {
  return {
    name: "Sheet",
    kind: "page",
    title: "Sheet",
    preBlock,
    roots,
    format: "md",
    readOnly: false,
    guide: false,
  };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function doubleClick(target: EventTarget): MouseEvent {
  const event = new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 });
  target.dispatchEvent(event);
  return event;
}

function pointer(type: string, x: number, y: number, init: Partial<MouseEventInit> = {}): Event {
  return new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: y, ...init });
}

function keydown(target: EventTarget, key: string): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function pointerEnter(target: EventTarget): Event {
  const event = new Event("pointerenter", { bubbles: false, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function pointerLeave(target: EventTarget): Event {
  const event = new Event("pointerleave", { bubbles: false, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function fieldHeader(root: HTMLElement, label: string): HTMLElement {
  const header = [...root.querySelectorAll<HTMLElement>(".sheet-field-header")].find((el) => el.textContent?.trim() === label);
  if (!header) throw new Error(`missing ${label} field header`);
  return header;
}

function dragFieldHeader(source: HTMLElement, target: HTMLElement, before = true): void {
  const elementFromPoint = document.elementFromPoint;
  document.elementFromPoint = () => target;
  try {
    source.dispatchEvent(pointer("pointerdown", 0, 0));
    const x = before ? -10 : 10;
    window.dispatchEvent(pointer("pointermove", x, 0));
    window.dispatchEvent(pointer("pointerup", x, 0));
  } finally {
    document.elementFromPoint = elementFromPoint;
  }
}

function cell(root: HTMLElement, row: number, col: number, gridId = "table"): HTMLElement {
  const el = root.querySelector(
    `.sheet-cell[data-sheet-grid-id="${gridId}"][data-row="${row}"][data-col="${col}"]`
  ) as HTMLElement | null;
  if (!el) throw new Error(`missing cell ${row},${col}`);
  return el;
}

function contextMenu(target: EventTarget): MouseEvent {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 30 });
  target.dispatchEvent(event);
  return event;
}

function input(target: EventTarget): Event {
  const event = new Event("input", { bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function clickMenuItem(label: string): void {
  const item = [...document.querySelectorAll(".ctx-item")].find((el) => el.textContent?.trim() === label) as
    | HTMLElement
    | undefined;
  if (!item) throw new Error(`missing menu item ${label}`);
  item.click();
}

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function loadTableDoc() {
  setDoc({
    byId: {
      table: node("table", "Table\ntine.view:: table", null, ["r1", "r2"]),
      r1: node("r1", "TODO [#A] Ship #sheets\nSCHEDULED: <2026-07-08 Wed>\nowner:: Martin", "table"),
      r2: node("r2", "DONE Verify\nDEADLINE: <2026-07-09 Thu>\nestimate:: 2h", "table"),
    },
    pages: [page(["table"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

describe("SheetTable", () => {
  it("routes real window Arrow keys from a clicked Table cell (GH #113)", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <SheetTable ownerId="table" rowSource="children" />);
    const disposeKeys = installKeybindings();
    try {
      cell(root, 0, 0).dispatchEvent(pointer("pointerdown", 5, 5));
      expect(cellSel()).toMatchObject({ gridId: "table", row: 0, col: 0 });

      const right = new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true });
      window.dispatchEvent(right);
      expect(right.defaultPrevented).toBe(true);
      expect(cellSel()).toMatchObject({ gridId: "table", row: 0, col: 1 });

      const down = new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true });
      window.dispatchEvent(down);
      expect(down.defaultPrevented).toBe(true);
      expect(cellSel()).toMatchObject({ gridId: "table", row: 1, col: 1 });
    } finally {
      disposeKeys();
      dispose();
    }
  });

  it("keeps a same-UUID journal twin DTO-only when the page twin is preloaded", async () => {
    const shared = "same-id";
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null),
        [shared]: { ...node(shared, "Loaded page row", null), page: "Twin" },
      },
      pages: [
        page(["table"]),
        { ...page([shared]), name: "Twin", title: "Twin", kind: "page" },
      ],
      feed: ["Sheet"], loaded: true,
    });
    const groups: RefGroup[] = [
      { page: "Twin", kind: "page", blocks: [{ id: shared, raw: "Page DTO", collapsed: false, children: [] }] },
      { page: "Twin", kind: "journal", blocks: [{ id: shared, raw: "Journal DTO", collapsed: false, children: [] }] },
    ];
    const { root, dispose } = mount(() => <SheetTable ownerId="table" rowSource="query" groups={groups} />);
    expect([...root.querySelectorAll(".sheet-title-cell .sheet-cell-body")].map((el) => el.textContent?.trim()))
      .toEqual(["Loaded page row", "Journal DTO"]);

    cell(root, 0, 0).dispatchEvent(pointer("pointerdown", 0, 0));
    const pageSelection = cellSel();
    expect(pageSelection?.kind === "cell" ? pageSelection.rowId : null).toBe(`page\0Twin\0${shared}`);
    expect(handleCellSelectionKey(new KeyboardEvent("keydown", { key: "ArrowDown" }))).toBe(true);
    const journalSelection = cellSel();
    expect(journalSelection?.kind === "cell" ? journalSelection.rowId : null).toBe(`journal\0Twin\0${shared}`);

    cell(root, 0, 0).dispatchEvent(pointer("pointerdown", 0, 0, { shiftKey: true }));
    const range = cellSel();
    expect(range?.kind).toBe("range");
    if (range?.kind === "range") {
      expect(range.anchorRowId).toBe(`journal\0Twin\0${shared}`);
      expect(range.focusRowId).toBe(`page\0Twin\0${shared}`);
    }

    expect(startCellEditing({ gridId: "table", surfaceId: "main", row: 1, col: 0 })).toBe(false);

    doubleClick(cell(root, 1, 0));
    await tick();
    expect(editingId()).toBeNull();
    expect(root.querySelector("textarea.block-editor")).toBeNull();
    expect(doc.byId[shared].raw).toBe("Loaded page row");
    dispose();
  });

  it("scopes one sheet rendered in two panes to the pane that started editing", async () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null, ["r1"]),
        r1: node("r1", "Alpha", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <>
      <SurfaceContext.Provider value="pane:left"><SheetTable ownerId="table" rowSource="children" /></SurfaceContext.Provider>
      <SurfaceContext.Provider value="pane:right"><SheetTable ownerId="table" rowSource="children" /></SurfaceContext.Provider>
    </>);
    const right = root.querySelector<HTMLElement>('[data-sheet-surface-id="pane:right"][data-row="0"][data-col="0"]')!;
    doubleClick(right);
    await Promise.resolve();
    expect(root.querySelectorAll("textarea.block-editor")).toHaveLength(1);
    expect(right.querySelector("textarea.block-editor")).not.toBeNull();
    dispose();
  });

  it("keeps a title editor attached to its row identity while sorting reorders rows", async () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null, ["a", "b"]),
        a: node("a", "Alpha", "table"),
        b: node("b", "Beta", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);
    (root.querySelector(".sheet-title-header") as HTMLElement).click();
    doubleClick(cell(root, 0, 0));
    const editor = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
    editor.value = "Zulu";
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "Zulu" }));
    await Promise.resolve();
    expect(editingId()).toBe("a");
    expect(root.querySelectorAll("textarea.block-editor")).toHaveLength(1);
    expect((root.querySelector("textarea.block-editor") as HTMLTextAreaElement).value).toBe("Zulu");
    dispose();
  });

  it("keeps sorted range endpoints attached to their row identities", async () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null, ["a", "b", "c"]),
        a: node("a", "Alpha", "table"),
        b: node("b", "Beta", "table"),
        c: node("c", "Charlie", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);
    (root.querySelector(".sheet-title-header") as HTMLElement).click();
    setCellRangeSelection("table", { row: 1, col: 0 }, { row: 0, col: 0 }, "main");
    const before = cellSel();
    expect(before?.kind === "range" ? before.anchorRowId : null).toBe("b");
    expect(before?.kind === "range" ? before.focusRowId : null).toBe("a");

    setRaw("a", "Zulu");
    await tick();

    const selected = cellSel();
    expect(selected?.kind).toBe("range");
    if (selected?.kind === "range") {
      expect(selected.anchor).toEqual({ row: 0, col: 0 });
      expect(selected.focus).toEqual({ row: 2, col: 0 });
      expect(selected.anchorRowId).toBe("b");
      expect(selected.focusRowId).toBe("a");
    }
    expect(cell(root, 2, 0).dataset.sheetRowId).toBe("a");
    expect(cell(root, 2, 0).classList.contains("sheet-cell-selected")).toBe(true);
    dispose();
  });

  it("keeps B selected and editable when schema removal shifts it into A's column", async () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: a=text;b=text;c=text", null, ["r1"]),
        r1: node("r1", "Row\na:: A\nb:: B\nc:: C", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);
    cell(root, 0, 2).dispatchEvent(pointer("pointerdown", 0, 0));
    const before = cellSel();
    expect(before?.kind === "cell" ? before.columnId : null).toBe("prop:b");

    setRaw("table", "Table\ntine.view:: table\ntine.fields:: b=text;c=text", { timetracking: false });
    await tick();

    const selected = cellSel();
    expect(selected?.kind === "cell" ? { col: selected.col, columnId: selected.columnId } : null)
      .toEqual({ col: 1, columnId: "prop:b" });
    handleCellSelectionKey(new KeyboardEvent("keydown", { key: "Enter" }));
    const input = root.querySelector("input.sheet-prop-input") as HTMLInputElement | null;
    expect(input?.value).toBe("B");
    dispose();
  });

  it("uses the cached row index for repeated navigation in a large query table", () => {
    const blocks = Array.from({ length: 5_000 }, (_, i) => ({
      id: `q${i}`,
      raw: `Row ${i}`,
      collapsed: false,
      children: [],
    }));
    setDoc({
      byId: { table: node("table", "Table\ntine.view:: table", null) },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    let indexed = 0;
    __sheetTableTestHooks.onIndexRow = () => indexed++;
    const { dispose } = mount(() => (
      <SheetTable ownerId="table" rowSource="query" groups={[{ page: "Remote", kind: "page", blocks }]} />
    ));
    setCellSel({ gridId: "table", surfaceId: "main", rowId: "page\0Remote\0q0", columnId: "title", row: 0, col: 0 });
    expect(indexed).toBe(5_000);
    for (let i = 0; i < 20; i++) handleCellSelectionKey(new KeyboardEvent("keydown", { key: "ArrowDown" }));
    expect(indexed).toBe(5_000);
    dispose();
  });

  it("extends the query window before a refreshed off-window range can edit", async () => {
    const filler = Array.from({ length: 270 }, (_, i) => ({ id: `f${i}`, raw: `Row ${i}`, collapsed: false, children: [] }));
    const a = { id: "live-a", raw: "Alpha", collapsed: false, children: [] };
    const b = { id: "live-b", raw: "Beta", collapsed: false, children: [] };
    const remote: FeedPage = {
      name: "Remote", kind: "page", title: "Remote", preBlock: null, roots: [a.id, b.id],
      format: "md", readOnly: false, guide: false,
    };
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null),
        [a.id]: { ...node(a.id, a.raw, null), page: remote.name },
        [b.id]: { ...node(b.id, b.raw, null), page: remote.name },
      },
      pages: [page(["table"]), remote], feed: ["Sheet"], loaded: true,
    });
    const group = (blocks: typeof filler): RefGroup[] => [{ page: remote.name, kind: "page", blocks }];
    const [groups, setGroups] = createSignal(group([a, b, ...filler]));
    const { root, dispose } = mount(() => <SheetTable ownerId="table" rowSource="query" groups={groups()} />);
    setCellRangeSelection("table", { row: 0, col: 0 }, { row: 1, col: 0 }, "main");

    setGroups(group([...filler.slice(0, 250), a, ...filler.slice(250, 259), b, ...filler.slice(259)]));

    const selected = cellSel();
    expect(selected?.kind === "range" ? selected.anchor.row : -1).toBe(250);
    expect(selected?.kind === "range" ? selected.focus.row : -1).toBe(260);
    expect(root.querySelector('[data-row="260"][data-col="0"]')).not.toBeNull();
    expect(handleCellSelectionKey(new KeyboardEvent("keydown", { key: "Enter" }))).toBe(true);
    await tick();
    expect(root.querySelector('[data-row="260"] textarea.block-editor')).not.toBeNull();
    dispose();
  });

  it("uses the 100k row index for repeated cellForBlock nested-ascent lookups without rescanning", () => {
    const target = "live-target";
    const blocks = Array.from({ length: 100_000 }, (_, i) => ({
      id: i === 0 ? target : `q${i}`,
      raw: `Row ${i}`,
      collapsed: false,
      children: [],
    }));
    const remote: FeedPage = {
      name: "Remote", kind: "page", title: "Remote", preBlock: null, roots: [target],
      format: "md", readOnly: false, guide: false,
    };
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null),
        [target]: { ...node(target, "Row 0", null), page: remote.name },
      },
      pages: [page(["table"]), remote], feed: ["Sheet"], loaded: true,
    });
    let indexed = 0;
    __sheetTableTestHooks.onIndexRow = () => indexed++;
    const { dispose } = mount(() => (
      <SheetTable ownerId="table" rowSource="query" groups={[{ page: remote.name, kind: "page", blocks }]} />
    ));
    expect(indexed).toBe(100_000);
    for (let i = 0; i < 50; i++) {
      expect(cellForBlockId(target, "main")?.rowId).toBe(`page\0Remote\0${target}`);
    }
    expect(indexed).toBe(100_000);
    dispose();
  });

  it("does not navigate selection into rows beyond the rendered window", () => {
    const ids = Array.from({ length: 201 }, (_, i) => `r${i}`);
    const byId: Record<string, StoreNode> = { table: node("table", "Table\ntine.view:: table", null, ids) };
    for (const [i, id] of ids.entries()) byId[id] = node(id, `Row ${i}`, "table");
    setDoc({ byId, pages: [page(["table"])], feed: ["Sheet"], loaded: true });
    const { root, dispose } = mount(() => <Block id="table" />);
    setCellSel({ gridId: "table", row: 199, col: 0 });
    handleCellSelectionKey(new KeyboardEvent("keydown", { key: "ArrowDown" }));
    const selected = cellSel();
    expect(selected?.kind === "cell" ? selected.row : -1).not.toBe(200);
    expect(root.querySelector('[data-row="200"]')).toBeNull();
    dispose();
  });
  it("renders children rows with observed field columns in stable order", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);

    const headers = [...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim());
    expect(headers).toEqual(["Block", "State", "Priority", "Scheduled", "Deadline", "Tags", "owner", "estimate", "+Add column"]);
    expect(root.querySelector(".sheet-col-stray")).toBeNull();
    expect(root.querySelector(".sheet-title-header")?.classList.contains("sheet-sticky-left")).toBe(true);
    expect(root.querySelector(".sheet-title-cell")?.classList.contains("sheet-sticky-left")).toBe(true);
    expect(root.querySelector(".sheet-field-header")?.classList.contains("sheet-sticky-left")).toBe(false);

    dispose();
  });

  it("single-click selects table cells, double-click edits the title, and dragging selects a range", async () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);

    cell(root, 0, 0).dispatchEvent(pointer("pointerdown", 0, 0));
    expect(cellSel()).toEqual({ kind: "cell", gridId: "table", row: 0, col: 0 });
    expect(cell(root, 0, 0).classList.contains("sheet-sticky-left")).toBe(true);
    expect(cell(root, 0, 0).classList.contains("sheet-cell-selected")).toBe(true);
    expect(editingId()).toBeNull();

    cell(root, 0, 1).dispatchEvent(pointer("pointerdown", 0, 0));
    expect(cellSel()).toEqual({ kind: "cell", gridId: "table", row: 0, col: 1 });
    expect(editingId()).toBeNull();

    doubleClick(cell(root, 0, 0));
    await tick();
    expect(editingId()).toBe("r1");

    resetCellSelectionForTests();
    const prevElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => cell(root, 1, 2);
    cell(root, 0, 1).dispatchEvent(pointer("pointerdown", 0, 0));
    window.dispatchEvent(pointer("pointermove", 8, 8));
    window.dispatchEvent(pointer("pointerup", 8, 8));
    document.elementFromPoint = prevElementFromPoint;

    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "table",
      anchor: { row: 0, col: 1 },
      focus: { row: 1, col: 2 },
    });
    expect(cell(root, 0, 1).classList.contains("sheet-cell-in-range")).toBe(true);
    expect(cell(root, 1, 2).classList.contains("sheet-cell-selected")).toBe(true);

    setCellSel({ gridId: "table", row: 0, col: 0 });
    cell(root, 1, 1).dispatchEvent(pointer("pointerdown", 0, 0, { shiftKey: true }));
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "table",
      anchor: { row: 0, col: 0 },
      focus: { row: 1, col: 1 },
    });

    dispose();
  });

  it("keeps table column tracks unchanged when entering title edit", async () => {
    const originalRect = HTMLElement.prototype.getBoundingClientRect;
    const rect = (width: number): DOMRect => ({
      x: 0,
      y: 0,
      left: 0,
      top: 0,
      right: width,
      bottom: 30,
      width,
      height: 30,
      toJSON: () => ({}),
    } as DOMRect);
    HTMLElement.prototype.getBoundingClientRect = function () {
      if (this.classList.contains("sheet-title-header")) return rect(180);
      if (this.classList.contains("sheet-field-header")) return rect(92);
      if (this.classList.contains("sheet-add-field")) return rect(96);
      if (this.classList.contains("sheet-title-cell")) return rect(180);
      if (this.classList.contains("sheet-field-cell")) return rect(92);
      if (this.classList.contains("sheet-row-tail")) return rect(96);
      return originalRect.call(this);
    };
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);
    try {
      const table = root.querySelector(".sheet-table") as HTMLElement;
      cell(root, 0, 0).dispatchEvent(pointer("pointerdown", 0, 0));
      await tick();
      const before = table.style.gridTemplateColumns;

      doubleClick(cell(root, 0, 0));
      await tick();

      expect(editingId()).toBe("r1");
      expect(table.style.gridTemplateColumns).toBe(before);
    } finally {
      dispose();
      HTMLElement.prototype.getBoundingClientRect = originalRect;
    }
  });

  it("renders declared schema fields first, keeps empty declared columns, and marks strays", () => {
    setDoc({
      byId: {
        table: node(
          "table",
          "Table\ntine.view:: table\ntine.fields:: status=enum:todo,doing;owner=text;missing=number",
          null,
          ["r1", "r2"]
        ),
        r1: node("r1", "Row one\nstatus:: todo\nstray:: typo", "table"),
        r2: node("r2", "Row two\nowner:: Martin", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    const headers = [...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim());
    expect(headers).toEqual(["Block", "status", "owner", "missing", "stray", "+Add column"]);
    expect(cell(root, 0, 3).textContent?.replace("⋮", "").trim()).toBe("");
    const stray = [...root.querySelectorAll(".sheet-field-header")].find((h) => h.textContent?.trim() === "stray");
    expect(stray?.classList.contains("sheet-col-stray")).toBe(true);

    dispose();
  });

  it("uses schemaPage fields when the owner block has no schema", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null, ["r1"]),
        r1: node("r1", "Tagged row\nstatus:: todo", "table"),
      },
      pages: [page(["table"], "tine.fields:: status=enum:todo,done;owner=text")],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <SheetTable ownerId="table" rowSource="children" schemaPage="Sheet" />);

    const headers = [...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim());
    expect(headers).toEqual(["Block", "status", "owner", "+Add column"]);

    dispose();
  });

  it("declaring the first prop field seeds the full observed field order", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null, ["r1"]),
        r1: node("r1", "TODO [#A] Task\nowner:: Martin\nestimate:: 2", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
      </>
    ));
    const estimateHeader = [...root.querySelectorAll(".sheet-field-header")].find((h) =>
      h.textContent?.includes("estimate")
    ) as HTMLElement | undefined;
    expect(estimateHeader).not.toBeUndefined();

    contextMenu(estimateHeader!);
    clickMenuItem("Declare field (text)");

    expect(blockProperty("table", "tine.fields")).toBe("state=state;priority=priority;owner=text;estimate=text");
    expect(doc.byId.table.raw).toBe(
      "Table\ntine.view:: table\ntine.fields:: state=state;priority=priority;owner=text;estimate=text"
    );
    dispose();
  });

  it("materializes the current visual schema before reordering a first property drag", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null, ["r1"]),
        r1: node("r1", "TODO Task\nowner:: Martin\nestimate:: 2", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const initial = mount(() => <SheetTable ownerId="table" rowSource="children" />);

    dragFieldHeader(fieldHeader(initial.root, "estimate"), fieldHeader(initial.root, "owner"));

    expect(blockProperty("table", "tine.fields")).toBe("state=state;estimate=text;owner=text");
    expect([...initial.root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim()))
      .toEqual(["Block", "State", "estimate", "owner", "+Add column"]);
    initial.dispose();

    const restarted = mount(() => <SheetTable ownerId="table" rowSource="children" />);
    expect([...restarted.root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim()))
      .toEqual(["Block", "State", "estimate", "owner", "+Add column"]);
    restarted.dispose();
  });

  it("reorders declared fields without absorbing undeclared columns into the schema", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: b=number;a=text", null, ["r1"]),
        r1: node("r1", "Row\na:: first\nb:: 2\nx:: kept\ny:: kept too", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <SheetTable ownerId="table" rowSource="children" />);

    dragFieldHeader(fieldHeader(root, "a"), fieldHeader(root, "b"));

    expect(blockProperty("table", "tine.fields")).toBe("a=text;b=number");
    expect([...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim()))
      .toEqual(["Block", "a", "b", "x", "y", "+Add column"]);
    dispose();
  });

  it("keeps formula columns pinned after a property is dropped on them", () => {
    setDoc({
      byId: {
        table: node(
          "table",
          "Table\ntine.view:: table\ntine.fields:: a=text;b=text\ntine.formula.total:: a + b",
          null,
          ["r1"]
        ),
        r1: node("r1", "Row\na:: 1\nb:: 2", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <SheetTable ownerId="table" rowSource="children" />);

    dragFieldHeader(fieldHeader(root, "a"), fieldHeader(root, "ƒtotal"), false);

    expect(blockProperty("table", "tine.fields")).toBe("b=text;a=text");
    expect([...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim()))
      .toEqual(["Block", "b", "a", "ƒtotal", "+Add column"]);
    dispose();
  });

  it("round-trips reordered schemas through block and page schema homes", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: first=text;second=number", null, ["r1"]),
        r1: node("r1", "Row\nfirst:: one\nsecond:: 2", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const block = mount(() => <SheetTable ownerId="table" rowSource="children" />);
    dragFieldHeader(fieldHeader(block.root, "second"), fieldHeader(block.root, "first"));
    expect(blockProperty("table", "tine.fields")).toBe("second=number;first=text");
    block.dispose();

    const blockRestarted = mount(() => <SheetTable ownerId="table" rowSource="children" />);
    expect([...blockRestarted.root.querySelectorAll(".sheet-field-header")].map((h) => h.textContent?.trim()))
      .toEqual(["second", "first"]);
    blockRestarted.dispose();
    document.body.innerHTML = "";

    setDoc({
      byId: { row: node("row", "Tagged #Tag\nfirst:: one\nsecond:: 2", null) },
      pages: [page(["row"], "tine.fields:: first=text;second=number")], feed: ["Sheet"], loaded: true,
    });
    const groups: RefGroup[] = [{
      page: "Sheet",
      kind: "page",
      blocks: [{
        id: "row",
        raw: doc.byId.row.raw,
        collapsed: false,
        children: [],
        tags: ["Tag"],
        properties: [["first", "one"], ["second", "2"]],
      }],
    }];
    const pageSchema = () => <SheetTable ownerId="tag-page:Tag" rowSource="query" groups={groups} schemaPage="Sheet" />;
    const pageHome = mount(pageSchema);
    dragFieldHeader(fieldHeader(pageHome.root, "second"), fieldHeader(pageHome.root, "first"));
    expect(readPageProperty("Sheet", "tine.fields")).toBe("second=number;first=text");
    pageHome.dispose();

    const pageRestarted = mount(pageSchema);
    expect([...pageRestarted.root.querySelectorAll(".sheet-field-header")].map((h) => h.textContent?.trim()).slice(0, 2))
      .toEqual(["second", "first"]);
    pageRestarted.dispose();
  });

  it("keeps a header click as a sort when pointer movement stays below the drag threshold", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: score=number", null, ["r1", "r2"]),
        r1: node("r1", "Beta\nscore:: 2", "table"),
        r2: node("r2", "Alpha\nscore:: 1", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <SheetTable ownerId="table" rowSource="children" />);
    const score = fieldHeader(root, "score");

    score.dispatchEvent(pointer("pointerdown", 0, 0));
    window.dispatchEvent(pointer("pointermove", 2, 0));
    window.dispatchEvent(pointer("pointerup", 2, 0));
    score.click();

    expect([...root.querySelectorAll(".sheet-title-cell .sheet-cell-body")].map((cell) => cell.textContent?.trim()))
      .toEqual(["Alpha", "Beta"]);
    dispose();
  });

  it("field header menu changes declared prop types and removes schema entries", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: owner=text;state=state", null, ["r1"]),
        r1: node("r1", "TODO Task\nowner:: Martin", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
      </>
    ));
    const ownerHeader = [...root.querySelectorAll(".sheet-field-header")].find((h) =>
      h.textContent?.includes("owner")
    ) as HTMLElement | undefined;
    const stateHeader = [...root.querySelectorAll(".sheet-field-header")].find((h) =>
      h.textContent?.includes("State")
    ) as HTMLElement | undefined;

    contextMenu(ownerHeader!);
    clickMenuItem("number");
    expect(doc.byId.table.raw).toBe("Table\ntine.view:: table\ntine.fields:: owner=number;state=state");

    contextMenu(ownerHeader!);
    clickMenuItem("Remove from schema");
    expect(doc.byId.table.raw).toBe("Table\ntine.view:: table\ntine.fields:: state=state");

    contextMenu(stateHeader!);
    clickMenuItem("Remove from schema");
    expect(doc.byId.table.raw).toBe("Table\ntine.view:: table");

    dispose();
  });

  it("offers field rename for a declared children-backed property (GH #175)", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: occurrence=number", null, ["r1"]),
        r1: node("r1", "Row\noccurrence:: 2", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
      </>
    ));
    const header = [...root.querySelectorAll(".sheet-field-header")].find((h) =>
      h.textContent?.includes("occurrence")
    ) as HTMLElement;

    contextMenu(header);
    expect([...document.querySelectorAll(".ctx-item")].map((el) => el.textContent?.trim())).toContain("Rename field…");

    dispose();
  });

  it("renames the GH #175 schema, rows, formulas, filter, grouping, and aggregates as one Undo unit", async () => {
    const ownerBefore = [
      "Table",
      "tine.view:: table",
      "tine.fields:: severity=number;occurrence=number;detection=number",
      'tine.formula.rpn:: severity * occurrence * detection + if(label == "occurrence", formula.occurrence, 0)',
      "tine.filter:: occurrence > 1",
      "tine.group-by:: prop:occurrence",
      "tine.col-aggregates:: prop:occurrence=sum;prop:severity=max",
    ].join("\n");
    const rowBefore = "Row\nseverity:: 2\noccurrence:: 2\ndetection:: 2\nlabel:: other";
    setDoc({
      byId: {
        table: node("table", ownerBefore, null, ["r1"]),
        r1: node("r1", rowBefore, "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
      </>
    ));
    const header = [...root.querySelectorAll(".sheet-field-header")].find((h) =>
      h.textContent?.includes("occurrence")
    ) as HTMLElement;

    contextMenu(header);
    clickMenuItem("Rename field…");
    const rename = root.querySelector<HTMLInputElement>('.sheet-header-rename-input[aria-label="Rename occurrence field"]')!;
    rename.value = "OCC";
    input(rename);
    keydown(rename, "Enter");
    await tick();

    const headersAfter = [...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim());
    expect(headersAfter.filter((label) => label === "OCC")).toHaveLength(1);
    expect(headersAfter).not.toContain("occurrence");
    expect(doc.byId.table.raw).toContain("tine.fields:: severity=number;OCC=number;detection=number");
    expect(doc.byId.table.raw).toContain('tine.formula.rpn:: severity * OCC * detection + if(label == "occurrence", formula.occurrence, 0)');
    expect(doc.byId.table.raw).toContain("tine.filter:: OCC > 1");
    expect(doc.byId.table.raw).toContain("tine.group-by:: prop:OCC");
    expect(doc.byId.table.raw).toContain("tine.col-aggregates:: prop:OCC=sum;prop:severity=max");
    expect(doc.byId.table.raw).not.toContain("occurrence=number");
    expect(doc.byId.table.raw).not.toContain("severity * occurrence * detection");
    expect(doc.byId.table.raw).not.toContain("tine.filter:: occurrence > 1");
    expect(doc.byId.table.raw).not.toContain("prop:occurrence");
    expect(doc.byId.table.raw).toContain('if(label == "occurrence", formula.occurrence, 0)');
    expect(doc.byId.r1.raw).toBe("Row\nseverity:: 2\nOCC:: 2\ndetection:: 2\nlabel:: other");
    expect(cell(root, 0, 4).textContent?.trim()).toBe("8");

    undo();
    await tick();
    expect(doc.byId.table.raw).toBe(ownerBefore);
    expect(doc.byId.r1.raw).toBe(rowBefore);
    expect([...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim())).toContain("occurrence");
    expect(cell(root, 0, 4).textContent?.trim()).toBe("8");

    redo();
    await tick();
    expect(doc.byId.table.raw).toContain("tine.fields:: severity=number;OCC=number;detection=number");
    expect(doc.byId.table.raw).toContain('tine.formula.rpn:: severity * OCC * detection + if(label == "occurrence", formula.occurrence, 0)');
    expect(doc.byId.table.raw).toContain("tine.filter:: OCC > 1");
    expect(doc.byId.table.raw).toContain("tine.group-by:: prop:OCC");
    expect(doc.byId.table.raw).toContain("tine.col-aggregates:: prop:OCC=sum;prop:severity=max");
    expect(doc.byId.table.raw).not.toContain("occurrence=number");
    expect(doc.byId.table.raw).not.toContain("severity * occurrence * detection");
    expect(doc.byId.table.raw).not.toContain("tine.filter:: occurrence > 1");
    expect(doc.byId.table.raw).not.toContain("prop:occurrence");
    expect(doc.byId.r1.raw).toContain("OCC:: 2");
    expect(doc.byId.r1.raw).not.toContain("occurrence:: 2");
    expect([...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim()).filter((label) => label === "OCC"))
      .toHaveLength(1);
    expect(cell(root, 0, 4).textContent?.trim()).toBe("8");

    dispose();
  });

  it("starts the same rename editor on header double-click", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: occurrence=number", null, ["r1"]),
        r1: node("r1", "Row\noccurrence:: 2", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);
    const header = [...root.querySelectorAll(".sheet-field-header")].find((h) => h.textContent?.includes("occurrence"))!;
    doubleClick(header);
    const rename = root.querySelector<HTMLInputElement>('.sheet-header-rename-input[aria-label="Rename occurrence field"]')!;
    expect(rename.value).toBe("occurrence");
    rename.value = "OCC";
    keydown(rename, "Enter");
    expect(blockProperty("table", "tine.fields")).toBe("OCC=number");
    expect(blockProperty("r1", "OCC")).toBe("2");
    dispose();
  });

  it("explains disabled rename for query-backed and inherited-schema tables", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null, ["r1"]),
        r1: node("r1", "Row\noccurrence:: 2", "table"),
      },
      pages: [page(["table"], "tine.fields:: occurrence=number")], feed: ["Sheet"], loaded: true,
    });
    const inherited = mount(() => <>
      <SheetTable ownerId="table" rowSource="children" schemaPage="Sheet" />
      <ContextMenu />
    </>);
    const inheritedHeader = [...inherited.root.querySelectorAll(".sheet-field-header")].find((h) => h.textContent?.includes("occurrence"))!;
    contextMenu(inheritedHeader);
    expect([...document.querySelectorAll(".ctx-disabled")].map((el) => el.textContent?.trim()))
      .toContain("Rename field… (page-inherited fields cannot be renamed here)");
    inherited.dispose();

    document.body.innerHTML = "";
    setRaw("table", "Table\ntine.view:: table\ntine.fields:: occurrence=number", { timetracking: false });
    const groups: RefGroup[] = [{ page: "Sheet", kind: "page", blocks: [{
      id: "r1", raw: doc.byId.r1.raw, collapsed: false, children: [], properties: [["occurrence", "2"]],
    }] }];
    const query = mount(() => <>
      <SheetTable ownerId="table" rowSource="query" groups={groups} />
      <ContextMenu />
    </>);
    const queryHeader = [...query.root.querySelectorAll(".sheet-field-header")].find((h) => h.textContent?.includes("occurrence"))!;
    contextMenu(queryHeader);
    expect([...document.querySelectorAll(".ctx-disabled")].map((el) => el.textContent?.trim()))
      .toContain("Rename field… (only children-backed tables can rename fields)");
    query.dispose();
  });

  it("leaves every raw block byte-identical when late preflight finds an affected page-owned formula", () => {
    const ownerRaw = "Table\ntine.view:: table\ntine.fields:: occurrence=number";
    const rowRaw = "Row\noccurrence:: 2";
    setDoc({
      byId: { table: node("table", ownerRaw, null, ["r1"]), r1: node("r1", rowRaw, "table") },
      pages: [page(["table"], "tine.formula.rpn:: occurrence * 2")], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <>
      <SheetTable ownerId="table" rowSource="children" schemaPage="Sheet" />
      <ContextMenu />
    </>);
    const header = [...root.querySelectorAll(".sheet-field-header")].find((h) => h.textContent?.includes("occurrence"))!;
    contextMenu(header);
    clickMenuItem("Rename field…");
    const rename = root.querySelector<HTMLInputElement>(".sheet-header-rename-input")!;
    rename.value = "OCC";
    keydown(rename, "Enter");
    expect(doc.byId.table.raw).toBe(ownerRaw);
    expect(doc.byId.r1.raw).toBe(rowRaw);
    expect(pageByName("Sheet")?.preBlock).toBe("tine.formula.rpn:: occurrence * 2");
    dispose();
  });

  it("round-trips an Org field rename through parser-recognized drawers", () => {
    const ownerRaw = [
      "Table",
      ":PROPERTIES:",
      ":tine.view: table",
      ":tine.fields: occurrence=number",
      ":tine.formula.rpn: occurrence * 2",
      ":END:",
      "body",
    ].join("\n");
    const rowRaw = ["Row", ":PROPERTIES:", ":occurrence: 2", ":other: kept", ":END:", "body"].join("\n");
    setDoc({
      byId: { table: node("table", ownerRaw, null, ["r1"]), r1: node("r1", rowRaw, "table") },
      pages: [{ ...page(["table"]), format: "org" }], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <>
      <SheetTable ownerId="table" rowSource="children" />
      <ContextMenu />
    </>);
    const header = [...root.querySelectorAll(".sheet-field-header")].find((h) => h.textContent?.includes("occurrence"))!;
    contextMenu(header);
    clickMenuItem("Rename field…");
    const rename = root.querySelector<HTMLInputElement>(".sheet-header-rename-input")!;
    rename.value = "OCC";
    keydown(rename, "Enter");

    expect(doc.byId.table.raw).toBe(ownerRaw
      .replace(":tine.fields: occurrence=number", ":tine.fields: OCC=number")
      .replace(":tine.formula.rpn: occurrence * 2", ":tine.formula.rpn: OCC * 2"));
    expect(doc.byId.r1.raw).toBe(rowRaw.replace(":occurrence:", ":OCC:"));
    expect(blockProperty("r1", "OCC")).toBe("2");
    expect(blockProperty("r1", "other")).toBe("kept");
    dispose();
  });

  it("keeps one schema column across rename, Undo, and Redo after Add-column then declare", async () => {
    setDoc({
      byId: { table: node("table", "Table\ntine.view:: table", null, ["r1"]), r1: node("r1", "Row", "table") },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const { root, dispose } = mount(() => <>
      <Block id="table" />
      <ContextMenu />
    </>);
    (root.querySelector(".sheet-add-column-ghost") as HTMLButtonElement).click();
    const add = root.querySelector(".sheet-add-field-input") as HTMLInputElement;
    add.value = "occurrence";
    keydown(add, "Enter");
    let header = [...root.querySelectorAll(".sheet-field-header")].find((h) => h.textContent?.includes("occurrence"))!;
    contextMenu(header);
    clickMenuItem("Declare field (text)");
    header = [...root.querySelectorAll(".sheet-field-header")].find((h) => h.textContent?.includes("occurrence"))!;
    contextMenu(header);
    clickMenuItem("Rename field…");
    const rename = root.querySelector<HTMLInputElement>(".sheet-header-rename-input")!;
    rename.value = "OCC";
    keydown(rename, "Enter");
    const labels = () => [...root.querySelectorAll(".sheet-field-header")].map((h) => h.textContent?.trim());
    expect(labels().filter((label) => label === "OCC")).toHaveLength(1);

    undo();
    await tick();
    expect(labels().filter((label) => label === "occurrence")).toHaveLength(1);
    redo();
    await tick();
    expect(labels().filter((label) => label === "OCC")).toHaveLength(1);
    dispose();
  });

  it("field header menu writes tag-page schema homes", () => {
    setDoc({
      byId: {
        row: node("row", "Tagged #Tag\nowner:: Martin", null),
      },
      pages: [page(["row"], null)],
      feed: ["Sheet"],
      loaded: true,
    });
    const groups: RefGroup[] = [
      {
        page: "Sheet",
        kind: "page",
        blocks: [
          {
            id: "row",
            raw: doc.byId.row.raw,
            collapsed: false,
            children: [],
            tags: ["Tag"],
            properties: [["owner", "Martin"]],
          },
        ],
      },
    ];
    const { root, dispose } = mount(() => (
      <>
        <SheetTable ownerId="tag-page:Tag" rowSource="query" groups={groups} schemaPage="Sheet" />
        <ContextMenu />
      </>
    ));
    const ownerHeader = [...root.querySelectorAll(".sheet-field-header")].find((h) =>
      h.textContent?.includes("owner")
    ) as HTMLElement | undefined;

    contextMenu(ownerHeader!);
    clickMenuItem("Declare field (text)");

    expect(readPageProperty("Sheet", "tine.fields")).toBe("tags=tags;owner=text;page=page");
    dispose();
  });

  it("tag-only query rows render declared empty typed cells from the tag-page schema", () => {
    setDoc({
      byId: {
        row: node("row", "#Tag ", null),
      },
      pages: [page(["row"], "tine.fields:: done=checkbox;status=enum:todo,done;due=date")],
      feed: ["Sheet"],
      loaded: true,
    });
    const groups: RefGroup[] = [
      {
        page: "Sheet",
        kind: "page",
        blocks: [
          {
            id: "row",
            raw: doc.byId.row.raw,
            collapsed: false,
            children: [],
            tags: ["Tag"],
            properties: [],
          },
        ],
      },
    ];
    const { root, dispose } = mount(() => (
      <SheetTable ownerId="tag-page:Tag" rowSource="query" groups={groups} schemaPage="Sheet" />
    ));

    const headers = [...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim());
    expect(headers).toEqual(["Block", "done", "status", "due", "Tags", "Page"]);
    expect(cell(root, 0, 1, "tag-page:Tag").textContent?.replace("⋮", "").trim()).toBe("");
    expect(cell(root, 0, 2, "tag-page:Tag").querySelector(".sheet-tag-chip")).toBeNull();
    expect(cell(root, 0, 3, "tag-page:Tag").querySelector(".date-chip")).toBeNull();

    dispose();
  });

  it("renders declared prop types on the read side without changing the editor", () => {
    setDoc({
      byId: {
        table: node(
          "table",
          [
            "Table",
            "tine.view:: table",
            "tine.fields:: done=checkbox;amount=number;due=date;starts=datetime;status=enum:todo,done;items=list;assignee=ref;badDate=date;badStatus=enum:a,b;badFlag=checkbox;badRef=ref",
          ].join("\n"),
          null,
          ["r1"]
        ),
        r1: node(
          "r1",
          [
            "Typed row",
            "done:: TRUE",
            "amount:: 42",
            "due:: 2026-07-08",
            "starts:: 2026-07-08 09:30",
            "status:: todo",
            "items:: alpha, beta",
            "assignee:: [[Alice]]",
            "badDate:: someday",
            "badStatus:: c",
            "badFlag:: maybe",
            "badRef:: Alice",
          ].join("\n"),
          "table"
        ),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    const checkbox = cell(root, 0, 1).querySelector('input[type="checkbox"]') as HTMLInputElement | null;
    expect(checkbox?.disabled).toBe(false);
    expect(checkbox?.readOnly).toBe(true);
    expect(checkbox?.checked).toBe(true);
    expect(cell(root, 0, 2).classList.contains("sheet-number-cell")).toBe(true);
    expect(cell(root, 0, 3).querySelector(".date-chip")?.textContent).toBe("2026-07-08");
    expect(cell(root, 0, 4).querySelector(".date-chip")?.textContent).toBe("2026-07-08 09:30");
    expect(cell(root, 0, 5).querySelector(".sheet-tag-chip")?.textContent).toBe("todo");
    expect([...cell(root, 0, 6).querySelectorAll(".sheet-tag-chip")].map((el) => el.textContent)).toEqual([
      "alpha",
      "beta",
    ]);
    expect(cell(root, 0, 7).querySelector(".page-ref")).not.toBeNull();
    expect(cell(root, 0, 8).querySelector(".date-chip")).toBeNull();
    expect(cell(root, 0, 8).textContent).toContain("someday");
    expect(cell(root, 0, 9).querySelector(".sheet-tag-chip")).toBeNull();
    expect(cell(root, 0, 9).textContent).toContain("c");
    expect(cell(root, 0, 10).querySelector('input[type="checkbox"]')).toBeNull();
    expect(cell(root, 0, 10).textContent).toContain("maybe");
    expect(cell(root, 0, 11).querySelector(".page-ref")).toBeNull();
    expect(cell(root, 0, 11).textContent).toContain("Alice");

    dispose();
  });

  it("state marker click cycles the marker, selects the cell, and does not edit", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);

    (cell(root, 0, 1).querySelector(".block-marker") as HTMLElement).click();

    expect(doc.byId.r1.raw.split("\n")[0]).toBe("DOING [#A] Ship #sheets");
    expect(cellSel()).toEqual({ kind: "cell", gridId: "table", row: 0, col: 1 });
    expect(cell(root, 0, 1).classList.contains("sheet-cell-selected")).toBe(true);
    expect(editingId()).toBeNull();
    dispose();
  });

  it("prop cell inline edit writes the property directly after the first line", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null, ["r1"]),
        r1: node("r1", "Task\nbody line\nowner:: old", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    doubleClick(cell(root, 0, 1));
    const input = root.querySelector("input.sheet-prop-input") as HTMLInputElement | null;
    expect(input).not.toBeNull();
    input!.value = "new";
    keydown(input!, "Enter");

    expect(doc.byId.r1.raw).toBe("Task\nowner:: new\nbody line");
    dispose();
  });

  it("commits a typed cell before Tab advances so the next value does not overwrite it (GH #176)", async () => {
    setDoc({
      byId: {
        table: node(
          "table",
          "Table\ntine.view:: table\ntine.fields:: severity=number;occurrence=number\ntine.formula.rpn:: severity * occurrence",
          null,
          ["r1"],
        ),
        r1: node("r1", "Risk", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    doubleClick(cell(root, 0, 1));
    const severity = root.querySelector("input.sheet-prop-input") as HTMLInputElement;
    severity.value = "2";
    input(severity);
    const tab = keydown(severity, "Tab");
    // jsdom does not perform native focus traversal. Mirror the browser blur
    // only when the component did not take ownership of the Tab gesture.
    if (!tab.defaultPrevented) severity.dispatchEvent(new FocusEvent("blur"));
    const afterTab = cellSel();

    expect(handleCellSelectionKey(new KeyboardEvent("keydown", { key: "3" }))).toBe(true);
    await tick();
    const nextDraft = root.querySelector("input.sheet-prop-input") as HTMLInputElement;
    expect(nextDraft.value).toBe("3");
    const nextWasFocused = document.activeElement === nextDraft;
    keydown(nextDraft, "Enter");

    // Assert the literal outcome only after the uninterrupted second edit. On
    // the pre-fix code, Tab left selection on severity and this exact sequence
    // produced `severity:: 3` with no occurrence value or formula result.
    expect(doc.byId.r1.raw).toBe("Risk\nseverity:: 2\noccurrence:: 3");
    expect(cell(root, 0, 3).textContent).toContain("6");
    expect(afterTab).toMatchObject({ kind: "cell", gridId: "table", row: 0, col: 2 });
    expect(nextWasFocused).toBe(true);

    dispose();
  });

  it("checkbox typed cells toggle true and false on single click", async () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: done=checkbox", null, ["r1"]),
        r1: node("r1", "Task", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    let checkbox = cell(root, 0, 1).querySelector('input[type="checkbox"]') as HTMLInputElement | null;
    expect(checkbox?.checked).toBe(false);
    checkbox!.click();
    expect(doc.byId.r1.raw).toBe("Task\ndone:: true");
    await tick();
    checkbox = cell(root, 0, 1).querySelector('input[type="checkbox"]') as HTMLInputElement | null;
    expect(checkbox?.checked).toBe(true);
    expect(cellSel()).toEqual({ kind: "cell", gridId: "table", row: 0, col: 1 });
    expect(editingId()).toBeNull();

    checkbox!.click();
    expect(doc.byId.r1.raw).toBe("Task\ndone:: false");
    await tick();

    dispose();
  });

  it("enum chip single-click opens the enum menu", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: status=enum:todo,doing", null, ["r1"]),
        r1: node("r1", "Task\nstatus:: todo", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
      </>
    ));

    (cell(root, 0, 1).querySelector(".sheet-tag-chip") as HTMLElement).click();

    expect(cellSel()).toEqual({ kind: "cell", gridId: "table", row: 0, col: 1 });
    expect(editingId()).toBeNull();
    expect([...document.querySelectorAll(".ctx-item")].map((el) => el.textContent?.trim())).toEqual([
      "todo",
      "doing",
      "Clear",
    ]);

    dispose();
  });

  it("enum typed cells open the shared popup, list declared values, write values, and clear", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: status=enum:todo,doing,done", null, ["r1"]),
        r1: node("r1", "Task\nstatus:: todo", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
      </>
    ));

    (cell(root, 0, 1).querySelector(".sheet-tag-chip") as HTMLElement).click();
    expect([...document.querySelectorAll(".ctx-item")].map((el) => el.textContent?.trim())).toEqual([
      "todo",
      "doing",
      "done",
      "Clear",
    ]);
    clickMenuItem("doing");
    expect(doc.byId.r1.raw).toBe("Task\nstatus:: doing");

    (cell(root, 0, 1).querySelector(".sheet-tag-chip") as HTMLElement).click();
    clickMenuItem("Clear");
    expect(doc.byId.r1.raw).toBe("Task");

    dispose();
  });

  it("number typed cells reject invalid input, accept decimals, and clear on empty input", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: amount=number", null, ["r1"]),
        r1: node("r1", "Task\namount:: 1", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    doubleClick(cell(root, 0, 1));
    let editor = root.querySelector("input.sheet-prop-input") as HTMLInputElement | null;
    expect(editor).not.toBeNull();
    editor!.value = "12abc";
    input(editor!);
    keydown(editor!, "Enter");
    expect(doc.byId.r1.raw).toBe("Task\namount:: 1");
    editor = root.querySelector("input.sheet-prop-input") as HTMLInputElement | null;
    expect(editor).not.toBeNull();
    expect(editor!.classList.contains("sheet-input-invalid")).toBe(true);

    // blur with invalid content must not commit or close (and must not throw
    // in the refocus microtask — regression: e.currentTarget nulls after dispatch)
    editor!.dispatchEvent(new FocusEvent("blur"));
    expect(doc.byId.r1.raw).toBe("Task\namount:: 1");
    expect(root.querySelector("input.sheet-prop-input")).not.toBeNull();

    editor!.value = "-3.5";
    input(editor!);
    keydown(editor!, "Enter");
    expect(doc.byId.r1.raw).toBe("Task\namount:: -3.5");
    expect(root.querySelector("input.sheet-prop-input")).toBeNull();

    doubleClick(cell(root, 0, 1));
    editor = root.querySelector("input.sheet-prop-input") as HTMLInputElement | null;
    editor!.value = "";
    input(editor!);
    keydown(editor!, "Enter");
    expect(doc.byId.r1.raw).toBe("Task");

    dispose();
  });

  it("prop date and datetime typed cells use the shared date picker and preserve datetime time", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: due=date;starts=datetime", null, ["r1"]),
        r1: node("r1", "Task\ndue:: 2026-07-08\nstarts:: 2026-07-08 09:30", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <DatePicker />
      </>
    ));

    (cell(root, 0, 1).querySelector(".date-chip") as HTMLElement).click();
    expect(root.querySelector(".date-picker")).not.toBeNull();
    const day10 = [...root.querySelectorAll(".date-picker .dp-cell")]
      .find((el) => el.textContent?.trim() === "10") as HTMLButtonElement | undefined;
    day10!.click();
    // Editing the `due` cell keeps `due` in its original (first) position — a
    // value edit must not reorder the columns (GH #216).
    expect(doc.byId.r1.raw).toBe("Task\ndue:: 2026-07-10\nstarts:: 2026-07-08 09:30");

    (cell(root, 0, 2).querySelector(".date-chip") as HTMLElement).click();
    const day11 = [...root.querySelectorAll(".date-picker .dp-cell")]
      .find((el) => el.textContent?.trim() === "11") as HTMLButtonElement | undefined;
    day11!.click();
    expect(doc.byId.r1.raw).toBe("Task\ndue:: 2026-07-10\nstarts:: 2026-07-11 09:30");

    dispose();
  });

  it("clicking an EMPTY date cell opens the picker so you can add a date to a row that lacks one", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.fields:: due=date", null, ["r1"]),
        r1: node("r1", "Task", "table"), // no due value
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <DatePicker />
      </>
    ));

    const dueCell = cell(root, 0, 1);
    expect(dueCell.querySelector(".date-chip")).toBeNull(); // empty → nothing to click before the fix
    dueCell.click();
    expect(root.querySelector(".date-picker")).not.toBeNull();
    const day15 = [...root.querySelectorAll(".date-picker .dp-cell")].find(
      (el) => el.textContent?.trim() === "15"
    ) as HTMLButtonElement | undefined;
    day15!.click();
    expect(doc.byId.r1.raw).toContain("due:: ");

    dispose();
  });

  it("cell menu 'Delete row' removes the row block in a table", () => {
    loadTableDoc(); // r1, r2
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
      </>
    ));

    contextMenu(cell(root, 0, 6)); // right-click r1's owner cell
    clickMenuItem("Delete row");
    expect(doc.byId.r1).toBeUndefined();
    expect(doc.byId.table.children).toEqual(["r2"]);

    dispose();
  });

  it("cell menu 'Delete column' removes the positional column in a grid", () => {
    setDoc({
      byId: {
        grid: node("grid", "Grid\ntine.view:: grid", null, ["gr1"]),
        gr1: node("gr1", "", "grid", ["ca", "cb"]),
        ca: node("ca", "A", "gr1"),
        cb: node("cb", "B", "gr1"),
      },
      pages: [page(["grid"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="grid" />
        <ContextMenu />
      </>
    ));

    contextMenu(cell(root, 0, 0, "grid")); // right-click cell A (col 0)
    clickMenuItem("Delete column");
    expect(doc.byId.ca).toBeUndefined();
    expect(doc.byId.gr1.children).toEqual(["cb"]);

    dispose();
  });

  it("sorts the table view without changing child order", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null, ["b", "a"]),
        b: node("b", "Beta", "table"),
        a: node("a", "Alpha", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    (root.querySelector(".sheet-title-header") as HTMLElement).click();

    const titles = [...root.querySelectorAll(".sheet-title-cell .sheet-cell-body")].map((c) => c.textContent?.trim());
    expect(titles).toEqual(["Alpha", "Beta"]);
    expect(doc.byId.table.children).toEqual(["b", "a"]);
    dispose();
  });

  it("children add-row creates an empty child at the end and enters title edit", async () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table", null, ["r1"]),
        r1: node("r1", "Existing", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    (root.querySelector(".sheet-add-row-ghost") as HTMLButtonElement).click();
    await tick();

    const children = doc.byId.table.children;
    expect(children).toHaveLength(2);
    const id = children[1];
    expect(doc.byId[id].raw).toBe("");
    expect(doc.byId[id].parent).toBe("table");
    expect(editingId()).toBe(id);
    expect(editingOwner()).toBe(`sheet:main:table:${id}:title`);

    dispose();
  });

  it("children add-column ghost adds an extra property column", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);

    (root.querySelector(".sheet-add-column-ghost") as HTMLButtonElement).click();
    const input = root.querySelector(".sheet-add-field-input") as HTMLInputElement;
    input.value = "reviewer";
    keydown(input, "Enter");

    const headers = [...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim());
    expect(headers).toContain("reviewer");

    dispose();
  });

  it("removes an added (extraFields-only) column via the header menu — no restart needed", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
      </>
    ));
    const headerLabels = () => [...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim());

    (root.querySelector(".sheet-add-column-ghost") as HTMLButtonElement).click();
    const input = root.querySelector(".sheet-add-field-input") as HTMLInputElement;
    input.value = "reviewer";
    keydown(input, "Enter");
    expect(headerLabels()).toContain("reviewer");

    // Regression: the added column lived only in the in-memory extraFields signal, so
    // removing it left it on screen until an app restart. It must vanish immediately.
    const reviewerHeader = [...root.querySelectorAll(".sheet-field-header")].find(
      (h) => h.textContent?.trim() === "reviewer"
    )!;
    contextMenu(reviewerHeader);
    clickMenuItem("Remove column");
    expect(headerLabels()).not.toContain("reviewer");

    dispose();
  });

  it("shows table add ghosts on hover without changing geometry", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);
    const table = root.querySelector(".sheet-table") as HTMLElement;
    const before = table.getBoundingClientRect();

    expect(root.querySelector(".sheet-add-column-ghost")).not.toBeNull();
    expect(root.querySelector(".sheet-add-row-ghost")).not.toBeNull();
    pointerEnter(table);
    const after = table.getBoundingClientRect();

    expect(after.left).toBe(before.left);
    expect(after.top).toBe(before.top);
    expect(after.width).toBe(before.width);
    expect(after.height).toBe(before.height);

    dispose();
  });

  it("renders a query-sourced table from existing query groups and gates off column add", () => {
    setDoc({
      byId: {
        q: node("q", "{{query (todo TODO)}}\ntine.view:: table", null),
        r1: node("r1", "TODO Query row\nowner:: Martin", null),
      },
      pages: [page(["q", "r1"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const groups: RefGroup[] = [
      { page: "Sheet", kind: "page", blocks: [{ id: "r1", raw: doc.byId.r1.raw, collapsed: false, children: [], marker: "TODO", properties: [["owner", "Martin"]] }] },
    ];

    const { root, dispose } = mount(() => <SheetTable ownerId="q" rowSource="query" groups={groups} />);

    expect(root.textContent).toContain("Query row");
    expect(root.textContent).toContain("Page");
    expect(root.querySelector(".sheet-add-column-ghost")).toBeNull();
    dispose();
  });

  it("renders a configured field aggregate footer", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.col-aggregates:: prop:estimate=sum", null, ["r1", "r2", "r3"]),
        r1: node("r1", "TODO First\nestimate:: 2h", "table"),
        r2: node("r2", "TODO Second\nestimate:: 5h", "table"),
        r3: node("r3", "DONE Third", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    expect(root.textContent).toContain("7 (1 skipped)");
    expect(root.querySelector(".sheet-aggregate-corner-toggle")).toBeNull();

    dispose();
  });

  it("filters table rows before sorting and aggregates", () => {
    setDoc({
      byId: {
        table: node(
          "table",
          "Table\ntine.view:: table\ntine.fields:: points=number\ntine.filter:: points > 2\ntine.col-aggregates:: prop:points=sum",
          null,
          ["r1", "r2", "r3"]
        ),
        r1: node("r1", "One\npoints:: 1", "table"),
        r2: node("r2", "Three\npoints:: 3", "table"),
        r3: node("r3", "Five\npoints:: 5", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    expect([...root.querySelectorAll(".sheet-title-cell .sheet-cell-body")].map((c) => c.textContent?.trim())).toEqual([
      "Three",
      "Five",
    ]);
    expect([...root.querySelectorAll(".sheet-aggregate-value")].map((el) => el.textContent?.trim())).toContain("8");

    dispose();
  });

  it("clears a selected range when filtering removes either stable row endpoint", async () => {
    setDoc({
      byId: {
        table: node(
          "table",
          "Table\ntine.view:: table\ntine.fields:: points=number\ntine.filter:: points > 2",
          null,
          ["r1", "r2", "r3"]
        ),
        r1: node("r1", "One\npoints:: 1", "table"),
        r2: node("r2", "Three\npoints:: 3", "table"),
        r3: node("r3", "Five\npoints:: 5", "table"),
      },
      pages: [page(["table"])], feed: ["Sheet"], loaded: true,
    });
    const { dispose } = mount(() => <Block id="table" />);
    setCellRangeSelection("table", { row: 0, col: 0 }, { row: 1, col: 0 }, "main");
    expect(cellSel()?.kind).toBe("range");

    setRaw("r2", "Three\npoints:: 0");
    await tick();

    expect(cellSel()).toBeNull();
    expect(handleCellSelectionKey(new KeyboardEvent("keydown", { key: "Enter" }))).toBe(false);
    expect(doc.byId.r3.raw).toBe("Five\npoints:: 5");
    dispose();
  });

  it("disables filtering and shows a warning chip on parse errors", () => {
    setDoc({
      byId: {
        table: node(
          "table",
          "Table\ntine.view:: table\ntine.fields:: points=number\ntine.filter:: points >",
          null,
          ["r1", "r2"]
        ),
        r1: node("r1", "One\npoints:: 1", "table"),
        r2: node("r2", "Three\npoints:: 3", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    expect([...root.querySelectorAll(".sheet-title-cell .sheet-cell-body")].map((c) => c.textContent?.trim())).toEqual([
      "One",
      "Three",
    ]);
    const chip = root.querySelector(".sheet-filter-error") as HTMLElement | null;
    expect(chip?.getAttribute("title")).toContain("Filter parse error");

    dispose();
  });

  it("disables filtering and shows a warning chip on non-boolean filter results", () => {
    setDoc({
      byId: {
        table: node(
          "table",
          "Table\ntine.view:: table\ntine.fields:: points=number\ntine.filter:: points * 2",
          null,
          ["r1", "r2"]
        ),
        r1: node("r1", "One\npoints:: 1", "table"),
        r2: node("r2", "Three\npoints:: 3", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    expect([...root.querySelectorAll(".sheet-title-cell .sheet-cell-body")].map((c) => c.textContent?.trim())).toEqual([
      "One",
      "Three",
    ]);
    const chip = root.querySelector(".sheet-filter-error") as HTMLElement | null;
    expect(chip?.getAttribute("title")).toContain("returned number");

    dispose();
  });

  it("renders computed formula columns as read-only typed cells with sort, aggregate, and remove", () => {
    setDoc({
      byId: {
        table: node(
          "table",
          [
            "Table",
            "tine.view:: table",
            "tine.fields:: price=number;qty=number",
            "tine.formula.total:: price * qty",
            'tine.formula.typed:: if(kind == "date", due + "1d", if(kind == "bool", true, missing + 1))',
            "tine.col-aggregates:: formula:total=sum",
          ].join("\n"),
          null,
          ["r1", "r2", "r3"]
        ),
        r1: node("r1", "Bool\nprice:: 5\nqty:: 2\nkind:: bool", "table"),
        r2: node("r2", "Date\nprice:: 1\nqty:: 2\nkind:: date\ndue:: 2026-07-08", "table"),
        r3: node("r3", "Error\nprice:: 2\nqty:: 3\nkind:: err", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
      </>
    ));

    const headers = [...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim());
    expect(headers).toEqual(["Block", "price", "qty", "ƒtotal", "ƒtyped", "kind", "due", "+Add column"]);
    expect(cell(root, 0, 3).classList.contains("sheet-number-cell")).toBe(true);
    expect(cell(root, 0, 3).textContent?.trim()).toBe("10");
    const bool = cell(root, 0, 4).querySelector('input[type="checkbox"]') as HTMLInputElement | null;
    expect(bool?.checked).toBe(true);
    expect(bool?.disabled).toBe(true);
    expect(cell(root, 1, 4).querySelector(".date-chip")?.textContent).toBe("2026-07-09");
    const error = cell(root, 2, 4).querySelector(".sheet-formula-error") as HTMLElement | null;
    expect(error?.getAttribute("title")).toContain("+ expects");
    expect([...root.querySelectorAll(".sheet-aggregate-value")].map((el) => el.textContent?.trim())).toContain("18");

    doubleClick(cell(root, 0, 3));
    expect(cell(root, 0, 3).classList.contains("sheet-cell-selected")).toBe(true);
    expect(root.querySelector("input.sheet-prop-input")).toBeNull();

    const totalHeader = [...root.querySelectorAll(".sheet-field-header")].find((h) =>
      h.textContent?.includes("total")
    ) as HTMLElement | undefined;
    totalHeader!.click();
    expect([...root.querySelectorAll(".sheet-title-cell .sheet-cell-body")].map((c) => c.textContent?.trim())).toEqual([
      "Date",
      "Error",
      "Bool",
    ]);

    contextMenu(totalHeader!);
    expect([...document.querySelectorAll(".ctx-item")].map((el) => el.textContent?.trim())).toEqual([
      "Rename field… (formula columns cannot be renamed here)",
      "Edit formula…",
      "Remove formula",
      "Add formula…", // a column header can also start a fresh formula column
    ]);
    clickMenuItem("Remove formula");
    expect(blockProperty("table", "tine.formula.total")).toBeNull();
    expect(doc.byId.table.raw).not.toContain("tine.formula.total::");
    expect(doc.byId.table.raw).toContain("tine.formula.typed::");

    dispose();
  });

  it("pins, unpins, and writes field aggregates from the corner toggle", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
      </>
    ));
    const table = root.querySelector(".sheet-table") as HTMLElement | null;
    const container = root.querySelector(".block-sheet-container") as HTMLElement | null;
    expect(root.querySelector(".sheet-footer-cell")).toBeNull();
    expect(root.querySelector(".sheet-aggregate-corner-toggle")).toBeNull();

    pointerEnter(container!);
    expect(root.querySelector(".sheet-footer-cell")).toBeNull();
    let toggle = root.querySelector(".sheet-aggregate-corner-toggle") as HTMLButtonElement | null;
    expect(toggle).not.toBeNull();

    toggle!.click();
    let footer = root.querySelector(".sheet-table > .sheet-footer-cell") as HTMLElement | null;
    expect(footer).not.toBeNull();
    expect(root.querySelector(".sheet-footer-overlay")).toBeNull();
    expect(footer!.style.position).not.toBe("absolute");
    toggle = root.querySelector(".sheet-aggregate-corner-toggle") as HTMLButtonElement | null;
    expect(toggle?.getAttribute("aria-pressed")).toBe("true");

    pointerLeave(table!);
    expect(root.querySelector(".sheet-table > .sheet-footer-cell")).not.toBeNull();
    toggle!.click();
    expect(root.querySelector(".sheet-footer-cell")).toBeNull();

    pointerEnter(container!);
    (root.querySelector(".sheet-aggregate-corner-toggle") as HTMLButtonElement).click();
    const adds = [...root.querySelectorAll(".sheet-table > .sheet-footer-cell .sheet-aggregate-add")] as HTMLButtonElement[];
    const estimateAdd = adds[adds.length - 1];
    expect(estimateAdd).not.toBeUndefined();
    estimateAdd.click();
    // In-DOM action menu, not a native <select> (whose WebKitGTK popup blurs
    // and collapses it — N27).
    const items = [...document.querySelectorAll(".ctx-item")];
    const sum = items.find((el) => el.textContent?.trim() === "Sum") as HTMLElement | undefined;
    expect(sum).toBeTruthy();

    pointerLeave(table!); // hover loss must not kill the menu or the pick
    sum!.click();

    expect(blockProperty("table", "tine.col-aggregates")).toBe("prop:estimate=sum");
    dispose();
  });

  it("edits scheduled cells through the shared date picker and can clear the planning line", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <DatePicker />
      </>
    ));

    (cell(root, 0, 3).querySelector(".date-chip") as HTMLElement).click();
    expect(root.querySelector(".date-picker")).not.toBeNull();
    const day10 = [...root.querySelectorAll(".date-picker .dp-cell")]
      .find((el) => el.textContent?.trim() === "10") as HTMLButtonElement | undefined;
    expect(day10).not.toBeUndefined();
    day10!.click();
    expect(doc.byId.r1.raw).toContain("SCHEDULED: <2026-07-10 Fri>");

    (cell(root, 0, 3).querySelector(".date-chip") as HTMLElement).click();
    (root.querySelector(".date-picker .dp-clear") as HTMLButtonElement).click();
    expect(doc.byId.r1.raw).not.toContain("SCHEDULED:");

    dispose();
  });

  it("recomputes a field aggregate after an inline cell edit", () => {
    setDoc({
      byId: {
        table: node("table", "Table\ntine.view:: table\ntine.col-aggregates:: prop:estimate=sum", null, ["r1", "r2", "r3"]),
        r1: node("r1", "TODO First\nestimate:: 2h", "table"),
        r2: node("r2", "TODO Second\nestimate:: 5h", "table"),
        r3: node("r3", "DONE Third", "table"),
      },
      pages: [page(["table"])],
      feed: ["Sheet"],
      loaded: true,
    });
    const { root, dispose } = mount(() => <Block id="table" />);

    expect(root.textContent).toContain("7 (1 skipped)");
    doubleClick(cell(root, 1, 2));
    const input = root.querySelector("input.sheet-prop-input") as HTMLInputElement | null;
    expect(input).not.toBeNull();
    input!.value = "8h";
    keydown(input!, "Enter");

    expect(root.textContent).toContain("10 (1 skipped)");
    dispose();
  });
});
