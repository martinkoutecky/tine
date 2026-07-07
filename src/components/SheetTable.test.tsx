import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { SheetTable } from "./SheetTable";
import { DatePicker } from "./DatePicker";
import { initParser } from "../render/parse";
import { blockProperty, doc, resetStore, setDoc, type FeedPage, type Node as StoreNode } from "../store";
import { setWorkflow } from "../ui";
import { resetCellSelectionForTests } from "../sheet/selection";
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

function mouseDown(target: EventTarget): MouseEvent {
  const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 });
  target.dispatchEvent(event);
  return event;
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

function cell(root: HTMLElement, row: number, col: number): HTMLElement {
  const el = root.querySelector(
    `.sheet-cell[data-sheet-grid-id="table"][data-row="${row}"][data-col="${col}"]`
  ) as HTMLElement | null;
  if (!el) throw new Error(`missing cell ${row},${col}`);
  return el;
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

  it("state cell click cycles the marker through the normal workflow", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);

    mouseDown(cell(root, 0, 1));

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

    mouseDown(cell(root, 0, 1));
    const input = root.querySelector("input.sheet-prop-input") as HTMLInputElement | null;
    expect(input).not.toBeNull();
    input!.value = "new";
    keydown(input!, "Enter");

    expect(doc.byId.r1.raw).toBe("Task\nowner:: new\nbody line");
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

    dispose();
  });

  it("writes the selected field aggregate token", () => {
    loadTableDoc();
    const { root, dispose } = mount(() => <Block id="table" />);
    const table = root.querySelector(".sheet-table") as HTMLElement | null;
    expect(root.querySelector(".sheet-footer-cell")).toBeNull();
    pointerEnter(table!);
    const adds = [...root.querySelectorAll(".sheet-footer-overlay .sheet-aggregate-add")] as HTMLButtonElement[];
    const estimateAdd = adds[adds.length - 1];
    expect(estimateAdd).not.toBeUndefined();
    estimateAdd.click();
    const estimateSelect = root.querySelector(".sheet-aggregate-select") as HTMLSelectElement | null;
    expect(estimateSelect).not.toBeNull();

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

    mouseDown(cell(root, 0, 3));
    expect(root.querySelector(".date-picker")).not.toBeNull();
    const day10 = [...root.querySelectorAll(".date-picker .dp-cell")]
      .find((el) => el.textContent?.trim() === "10") as HTMLButtonElement | undefined;
    expect(day10).not.toBeUndefined();
    day10!.click();
    expect(doc.byId.r1.raw).toContain("SCHEDULED: <2026-07-10 Fri>");

    mouseDown(cell(root, 0, 3));
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
    mouseDown(cell(root, 1, 2));
    const input = root.querySelector("input.sheet-prop-input") as HTMLInputElement | null;
    expect(input).not.toBeNull();
    input!.value = "8h";
    keydown(input!, "Enter");

    expect(root.textContent).toContain("10 (1 skipped)");
    dispose();
  });
});
