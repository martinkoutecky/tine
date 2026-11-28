// A table or board cell for property column `K` never changes a property line
// spelled differently, such as `ready::` under a `Ready` column: Logseq reads
// both as one property. The cell refuses visibly and writes nothing.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { Block } from "./Block";
import { ContextMenu } from "./ContextMenu";
import { DatePicker } from "./DatePicker";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { undoTopTag } from "../document/history";
import { type FeedPage, type Node as StoreNode } from "../document/model";
import { doc, setDoc } from "../document/model";
import { setWorkflow } from "../ui";
import { resetCellSelectionForTests } from "../sheet/selection";
import { setToasts, toasts } from "../toasts";

const saveFixture = vi.hoisted(() => ({ actual: null as Record<string, unknown> | null, markDirty: vi.fn() }));
vi.mock("../document/host/wiring", () => new Proxy({}, {
  get: (_target, key) => key === "then" ? undefined : key === "markDirty" ? saveFixture.markDirty
    : saveFixture.actual?.[key as string],
  has: () => true,
}));

beforeAll(async () => {
  saveFixture.actual = await vi.importActual("../document/host/wiring");
  await initParser();
});

afterEach(() => {
  resetCellSelectionForTests();
  resetStore();
  setToasts([]);
  setWorkflow("todo");
  saveFixture.markDirty.mockClear();
  document.body.innerHTML = "";
});

function page(roots: string[], format: "md" | "org" = "md"): FeedPage {
  return { name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots, format, readOnly: false, guide: false };
}

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

function load(byId: Record<string, StoreNode>, root: string, format: "md" | "org" = "md") {
  setDoc({ byId, pages: [page([root], format)], feed: ["Sheet"], loaded: true });
}

function mount(view: () => JSX.Element) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(view, host);
  return { root: host, dispose };
}

function cell(root: HTMLElement, row: number, field: string, gridId = "table"): HTMLElement {
  const headers = [...root.querySelectorAll<HTMLElement>("[data-sheet-field-header]")];
  const col = headers.findIndex((h) => h.dataset.sheetField === field) + 1;
  if (!col) throw new Error(`missing column ${field}`);
  const el = root.querySelector<HTMLElement>(`.sheet-cell[data-sheet-grid-id="${gridId}"][data-row="${row}"][data-col="${col}"]`);
  if (!el) throw new Error(`missing cell ${row},${field}`);
  return el;
}

function keydown(target: EventTarget, key: string) {
  target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

function typeInto(root: HTMLElement, target: HTMLElement, value: string) {
  target.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true, button: 0 }));
  const editor = root.querySelector<HTMLInputElement>("input.sheet-prop-input");
  if (!editor) throw new Error("no cell editor");
  editor.value = value;
  keydown(editor, "Enter");
}

function clickMenuItem(label: string) {
  const item = [...document.querySelectorAll<HTMLElement>(".ctx-item")].find((el) => el.textContent?.trim() === label);
  if (!item) throw new Error(`missing menu item ${label}`);
  item.click();
}

/** Nothing written: same raw, no undo entry, no dirty mark, the refusal shown. */
function expectRefused(id: string, raw: string, other: string, undoBefore: string | null) {
  expect(doc.byId[id].raw).toBe(raw);
  expect(undoTopTag()).toBe(undoBefore);
  expect(saveFixture.markDirty).not.toHaveBeenCalled();
  const shown = toasts().filter((t) => t.kind === "error").map((t) => t.message);
  expect(shown).toEqual([`Not changed: this row already has “${other}”, another spelling of this property.`]);
}

describe("a table cell refuses a write that would change another spelling of its property", () => {
  it.each([["\n"], ["\r\n"]])("typing in a Ready text column beside `ready:: false` (newline %j)", (nl) => {
    const raw = `Row${nl}ready:: false`;
    load({
      table: node("table", "Table\ntine.view:: table\ntine.fields:: ready=checkbox;Ready=text", null, ["r1"]),
      r1: node("r1", raw, "table"),
    }, "table");
    const { root, dispose } = mount(() => <Block id="table" />);
    const undoBefore = undoTopTag();
    typeInto(root, cell(root, 0, "prop:Ready"), "yes");
    expectRefused("r1", raw, "ready", undoBefore);
    dispose();
  });

  it.each([["Shipped", "shipped", "shipped:: true"], ["shipped", "Shipped", "Shipped:: true"], ["a_b", "a-b", "a-b:: false"], ["Ship_Ped", "ship-ped", "ship-ped:: true"]])(
    "clicking the %s checkbox beside `%s` (column orders swapped too)", (column, other, line) => {
      for (const fields of [`${column}=checkbox;${other}=checkbox`, `${other}=checkbox;${column}=checkbox`]) {
        const raw = `Row\n${line}`;
        load({
          table: node("table", `Table\ntine.view:: table\ntine.fields:: ${fields}`, null, ["r1"]),
          r1: node("r1", raw, "table"),
        }, "table");
        const { root, dispose } = mount(() => <Block id="table" />);
        const undoBefore = undoTopTag();
        cell(root, 0, `prop:${column}`).querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();
        expectRefused("r1", raw, other, undoBefore);
        // The other column still shows its own value.
        expect(cell(root, 0, `prop:${other}`).querySelector<HTMLInputElement>('input[type="checkbox"]')?.checked).toBe(line.endsWith("true"));
        dispose();
        resetStore();
        setToasts([]);
      }
    });

  it("a row holding `Ready:: yes` and `ready:: true`: the ready checkbox and clearing Ready keep both lines", () => {
    const raw = "Row\nReady:: yes\nready:: true";
    load({
      table: node("table", "Table\ntine.view:: table\ntine.fields:: ready=checkbox;Ready=text", null, ["r1"]),
      r1: node("r1", raw, "table"),
    }, "table");
    const { root, dispose } = mount(() => <Block id="table" />);
    const undoBefore = undoTopTag();
    cell(root, 0, "prop:ready").querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();
    expectRefused("r1", raw, "Ready", undoBefore);
    setToasts([]);
    typeInto(root, cell(root, 0, "prop:Ready"), "");
    expectRefused("r1", raw, "ready", undoBefore);
    dispose();
  });

  it("an observed column and an added column refuse beside another spelling", () => {
    const raw = "Row\nready:: false";
    load({
      table: node("table", "Table\ntine.view:: table", null, ["r1", "r2"]),
      r1: node("r1", raw, "table"),
      r2: node("r2", "Other\nReady:: yes", "table"),
    }, "table");
    const { root, dispose } = mount(() => <Block id="table" />);
    const undoBefore = undoTopTag();
    typeInto(root, cell(root, 0, "prop:Ready"), "no");
    expectRefused("r1", raw, "ready", undoBefore);
    dispose();

    resetStore();
    setToasts([]);
    load({
      table: node("table", "Table\ntine.view:: table\ntine.fields:: ready=checkbox", null, ["r1"]),
      r1: node("r1", raw, "table"),
    }, "table");
    const added = mount(() => <Block id="table" />);
    added.root.querySelector<HTMLElement>(".sheet-add-column-ghost")!.click();
    const name = added.root.querySelector<HTMLInputElement>("input.sheet-add-field-input")!;
    name.value = "Ready";
    keydown(name, "Enter");
    typeInto(added.root, cell(added.root, 0, "prop:Ready"), "no");
    expectRefused("r1", raw, "ready", undoTopTag());
    expect(doc.byId.r1.raw).toBe(raw);
    added.dispose();
  });

  it("the enum menu and the date picker refuse beside another spelling", () => {
    // The enum chip shows `status:: todo`; the row also holds `Status::`.
    const raw = "Row\nstatus:: todo\nStatus:: done\nDue:: 2026-07-08";
    load({
      table: node("table", "Table\ntine.view:: table\ntine.fields:: status=enum:todo,doing;due=date", null, ["r1"]),
      r1: node("r1", raw, "table"),
    }, "table");
    const { root, dispose } = mount(() => (
      <>
        <Block id="table" />
        <ContextMenu />
        <DatePicker />
      </>
    ));
    const undoBefore = undoTopTag();
    cell(root, 0, "prop:status").querySelector<HTMLElement>(".sheet-tag-chip")!.click();
    clickMenuItem("doing");
    expectRefused("r1", raw, "Status", undoBefore);
    setToasts([]);
    cell(root, 0, "prop:due").click();
    const day10 = [...root.querySelectorAll<HTMLElement>(".date-picker .dp-cell")].find((el) => el.textContent?.trim() === "10");
    day10!.click();
    expectRefused("r1", raw, "Due", undoBefore);
    dispose();
  });

  it("an Org drawer `:ready: false` under a Ready column is kept", () => {
    const raw = "Row\n:PROPERTIES:\n:ready: false\n:END:";
    load({
      table: node("table", "Table\n:PROPERTIES:\n:tine.view: table\n:tine.fields: ready=checkbox;Ready=text\n:END:", null, ["r1"]),
      r1: node("r1", raw, "table"),
    }, "table", "org");
    const { root, dispose } = mount(() => <Block id="table" />);
    const undoBefore = undoTopTag();
    typeInto(root, cell(root, 0, "prop:Ready"), "yes");
    expectRefused("r1", raw, "ready", undoBefore);
    dispose();
  });

  it("a board drag between property groups refuses a card holding another spelling", () => {
    const raw = "Card\nStatus:: doing";
    load({
      board: node("board", "Board\ntine.view:: board\ntine.group-by:: prop:status\ntine.fields:: status=enum:todo,doing", null, ["card", "other"]),
      card: node("card", raw, "board"),
      other: node("other", "Other\nstatus:: todo", "board"),
    }, "board");
    const { root, dispose } = mount(() => <Block id="board" />);
    const undoBefore = undoTopTag();
    const card = root.querySelector<HTMLElement>('[data-block-id="card"]')!;
    const target = root.querySelector<HTMLElement>('[data-board-col="0"]')!;
    const elementFromPoint = document.elementFromPoint;
    document.elementFromPoint = () => target;
    try {
      const at = (type: string, x: number) => new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX: x, clientY: 0 });
      card.dispatchEvent(at("pointerdown", 0));
      window.dispatchEvent(at("pointermove", 12));
      window.dispatchEvent(at("pointerup", 12));
    } finally {
      document.elementFromPoint = elementFromPoint;
    }
    expectRefused("card", raw, "Status", undoBefore);
    dispose();
  });
});

describe("cells without another spelling write as before", () => {
  it("checkbox, text and clear give develop's bytes", () => {
    load({
      table: node("table", "Table\ntine.view:: table\ntine.fields:: ready=checkbox;Ready=text", null, ["r1", "r2"]),
      r1: node("r1", "Row\nready:: false", "table"),
      r2: node("r2", "Plain", "table"),
    }, "table");
    const { root, dispose } = mount(() => <Block id="table" />);
    cell(root, 0, "prop:ready").querySelector<HTMLInputElement>('input[type="checkbox"]')!.click();
    expect(doc.byId.r1.raw).toBe("Row\nready:: true");
    typeInto(root, cell(root, 1, "prop:Ready"), "yes");
    expect(doc.byId.r2.raw).toBe("Plain\nReady:: yes");
    expect(toasts().filter((t) => t.kind === "error")).toEqual([]);
    dispose();
  });

  it("a declared `bad:key` column still shows the core's error text", () => {
    load({
      table: node("table", "Table\ntine.view:: table\ntine.fields:: bad:key=text", null, ["r1"]),
      r1: node("r1", "Row", "table"),
    }, "table");
    const { root, dispose } = mount(() => <Block id="table" />);
    typeInto(root, cell(root, 0, "prop:bad:key"), "x");
    expect(doc.byId.r1.raw).toBe("Row");
    expect(toasts().filter((t) => t.kind === "error").map((t) => t.message)).toEqual(["Invalid property edit"]);
    dispose();
  });
});
