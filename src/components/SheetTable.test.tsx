import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { SheetTable } from "./SheetTable";
import { initParser } from "../render/parse";
import { doc, resetStore, setDoc, type FeedPage, type Node as StoreNode } from "../store";
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

    const titles = [...root.querySelectorAll(".sheet-title-cell")].map((c) => c.textContent?.trim());
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
});
