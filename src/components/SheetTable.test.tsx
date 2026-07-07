import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { SheetTable } from "./SheetTable";
import { DatePicker } from "./DatePicker";
import { initParser } from "../render/parse";
import { blockProperty, doc, readPageProperty, resetStore, setDoc, type FeedPage, type Node as StoreNode } from "../store";
import { setWorkflow } from "../ui";
import { cellSel, resetCellSelectionForTests, setCellSel } from "../sheet/selection";
import { editingId, editingOwner } from "../editorController";
import type { RefGroup } from "../types";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
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
  };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function mouseDown(target: EventTarget, init: Partial<MouseEventInit> = {}): MouseEvent {
  const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0, ...init });
  target.dispatchEvent(event);
  return event;
}

function doubleClick(target: EventTarget): MouseEvent {
  const event = new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 });
  target.dispatchEvent(event);
  return event;
}

function pointer(type: string, x: number, y: number): Event {
  return new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: y });
}

function keydown(target: EventTarget, key: string): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function change(target: EventTarget): Event {
  const event = new Event("change", { bubbles: true, cancelable: true });
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
  it("renders children rows with observed field columns in stable order", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);

    const headers = [...root.querySelectorAll(".sheet-header-cell")].map((h) => h.textContent?.trim());
    expect(headers).toEqual(["Block", "State", "Priority", "Scheduled", "Deadline", "Tags", "owner", "estimate", "+"]);
    expect(root.querySelector(".sheet-col-stray")).toBeNull();
    expect(root.querySelector(".sheet-title-header")?.classList.contains("sheet-sticky-left")).toBe(true);
    expect(root.querySelector(".sheet-title-cell")?.classList.contains("sheet-sticky-left")).toBe(true);
    expect(root.querySelector(".sheet-field-header")?.classList.contains("sheet-sticky-left")).toBe(false);

    dispose();
  });

  it("single-click selects table cells, double-click edits the title, and dragging selects a range", async () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);

    mouseDown(cell(root, 0, 0));
    expect(cellSel()).toEqual({ kind: "cell", gridId: "table", row: 0, col: 0 });
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
    mouseDown(cell(root, 1, 1), { shiftKey: true });
    expect(cellSel()).toEqual({
      kind: "range",
      gridId: "table",
      anchor: { row: 0, col: 0 },
      focus: { row: 1, col: 1 },
    });

    dispose();
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
    expect(headers).toEqual(["Block", "status", "owner", "missing", "stray", "+"]);
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
    expect(headers).toEqual(["Block", "status", "owner", "+"]);

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
    expect(checkbox?.disabled).toBe(true);
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

  it("state cell double-click cycles the marker through the normal workflow", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);

    doubleClick(cell(root, 0, 1));

    expect(doc.byId.r1.raw.split("\n")[0]).toBe("DOING [#A] Ship #sheets");
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

  it("checkbox typed cells toggle true and false without enabling the rendered checkbox", () => {
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

    doubleClick(cell(root, 0, 1));
    expect(doc.byId.r1.raw).toBe("Task\ndone:: true");
    const checkbox = cell(root, 0, 1).querySelector('input[type="checkbox"]') as HTMLInputElement | null;
    expect(checkbox?.disabled).toBe(true);
    expect(checkbox?.checked).toBe(true);

    doubleClick(cell(root, 0, 1));
    expect(doc.byId.r1.raw).toBe("Task\ndone:: false");

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

    doubleClick(cell(root, 0, 1));
    expect([...document.querySelectorAll(".ctx-item")].map((el) => el.textContent?.trim())).toEqual([
      "todo",
      "doing",
      "done",
      "Clear",
    ]);
    clickMenuItem("doing");
    expect(doc.byId.r1.raw).toBe("Task\nstatus:: doing");

    doubleClick(cell(root, 0, 1));
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

    doubleClick(cell(root, 0, 1));
    expect(root.querySelector(".date-picker")).not.toBeNull();
    const day10 = [...root.querySelectorAll(".date-picker .dp-cell")]
      .find((el) => el.textContent?.trim() === "10") as HTMLButtonElement | undefined;
    day10!.click();
    expect(doc.byId.r1.raw).toBe("Task\nstarts:: 2026-07-08 09:30\ndue:: 2026-07-10");

    doubleClick(cell(root, 0, 2));
    const day11 = [...root.querySelectorAll(".date-picker .dp-cell")]
      .find((el) => el.textContent?.trim() === "11") as HTMLButtonElement | undefined;
    day11!.click();
    expect(doc.byId.r1.raw).toBe("Task\ndue:: 2026-07-10\nstarts:: 2026-07-11 09:30");

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

    (root.querySelector(".sheet-add-row-btn") as HTMLButtonElement).click();
    await tick();

    const children = doc.byId.table.children;
    expect(children).toHaveLength(2);
    const id = children[1];
    expect(doc.byId[id].raw).toBe("");
    expect(doc.byId[id].parent).toBe("table");
    expect(editingId()).toBe(id);
    expect(editingOwner()).toBe("sheet:table:1:0");

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
    expect(root.querySelector(".sheet-add-field-btn")).toBeNull();
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
    expect(headers).toEqual(["Block", "price", "qty", "ƒtotal", "ƒtyped", "kind", "due", "+"]);
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
      "Edit formula…",
      "Remove formula",
    ]);
    clickMenuItem("Remove formula");
    expect(blockProperty("table", "tine.formula.total")).toBeNull();
    expect(doc.byId.table.raw).not.toContain("tine.formula.total::");
    expect(doc.byId.table.raw).toContain("tine.formula.typed::");

    dispose();
  });

  it("pins, unpins, and writes field aggregates from the corner toggle", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);
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
    const estimateSelect = root.querySelector(".sheet-aggregate-select") as HTMLSelectElement | null;
    expect(estimateSelect).not.toBeNull();

    pointerLeave(table!);
    estimateSelect!.dispatchEvent(new FocusEvent("blur"));
    estimateSelect!.value = "sum";
    change(estimateSelect!);

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

    doubleClick(cell(root, 0, 3));
    expect(root.querySelector(".date-picker")).not.toBeNull();
    const day10 = [...root.querySelectorAll(".date-picker .dp-cell")]
      .find((el) => el.textContent?.trim() === "10") as HTMLButtonElement | undefined;
    expect(day10).not.toBeUndefined();
    day10!.click();
    expect(doc.byId.r1.raw).toContain("SCHEDULED: <2026-07-10 Fri>");

    doubleClick(cell(root, 0, 3));
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
