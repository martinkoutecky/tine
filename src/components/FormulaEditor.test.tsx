import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { FormulaEditor } from "./FormulaEditor";
import { initParser } from "../render/parse";
import { blockProperty, doc, resetStore, setDoc, type FeedPage, type Node as StoreNode } from "../store";
import { closeFormulaEditor, openFormulaEditor } from "../ui";
import { encodeFormulaExpr } from "../sheet/formula";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  closeFormulaEditor();
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

function loadEditorDoc(raw = "Table") {
  setDoc({
    byId: { table: node("table", raw, null) },
    pages: [page(["table"])],
    feed: ["Sheet"],
    loaded: true,
  });
}

function input(target: EventTarget): Event {
  const event = new Event("input", { bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function saveButton(root: ParentNode): HTMLButtonElement {
  const button = [...root.querySelectorAll(".formula-editor-btn")]
    .find((el) => el.textContent?.trim() === "Save") as HTMLButtonElement | undefined;
  if (!button) throw new Error("missing Save button");
  return button;
}

describe("FormulaEditor", () => {
  it("shows live parse errors and disables save", () => {
    loadEditorDoc();
    const { root, dispose } = mount(() => <FormulaEditor />);
    openFormulaEditor({
      mode: "add",
      ownerId: "table",
      x: 10,
      y: 10,
      expr: "",
      formulas: [],
      fields: ["points"],
    });

    const textarea = root.querySelector(".formula-editor-textarea") as HTMLTextAreaElement;
    textarea.value = "points >";
    input(textarea);

    expect([...root.querySelectorAll(".formula-editor-error")].map((el) => el.textContent).join("\n")).toContain(
      "Expected expression"
    );
    expect(saveButton(root).disabled).toBe(true);
    dispose();
  });

  it("writes an added formula as one armored property line", () => {
    loadEditorDoc();
    const { root, dispose } = mount(() => <FormulaEditor />);
    openFormulaEditor({
      mode: "add",
      ownerId: "table",
      x: 10,
      y: 10,
      expr: "",
      formulas: [],
      fields: ["points"],
    });
    const name = root.querySelector(".formula-editor-input") as HTMLInputElement;
    const textarea = root.querySelector(".formula-editor-textarea") as HTMLTextAreaElement;
    const expr = '"((x)" + "#tag"';

    name.value = "safe";
    input(name);
    textarea.value = expr;
    input(textarea);
    saveButton(root).click();

    expect(doc.byId.table.raw).toBe(`Table\ntine.formula.safe:: ${encodeFormulaExpr(expr)}`);
    expect(doc.byId.table.raw).toContain(String.raw`"( (x)" + "\#tag"`);
    dispose();
  });

  it("blocks duplicate names in add mode", () => {
    loadEditorDoc();
    const { root, dispose } = mount(() => <FormulaEditor />);
    openFormulaEditor({
      mode: "add",
      ownerId: "table",
      x: 10,
      y: 10,
      expr: "points",
      formulas: [["total", "points * 2"]],
      fields: ["points"],
    });

    const name = root.querySelector(".formula-editor-input") as HTMLInputElement;
    name.value = "total";
    input(name);

    expect(root.querySelector(".formula-editor-error")?.textContent).toContain("already exists");
    expect(saveButton(root).disabled).toBe(true);
    dispose();
  });

  it("writes and clears filter mode on the view block", () => {
    loadEditorDoc();
    const { root, dispose } = mount(() => <FormulaEditor />);
    openFormulaEditor({
      mode: "filter",
      ownerId: "table",
      x: 10,
      y: 10,
      expr: "",
      formulas: [],
      fields: ["points"],
    });
    let textarea = root.querySelector(".formula-editor-textarea") as HTMLTextAreaElement;
    textarea.value = "points > 2";
    input(textarea);
    saveButton(root).click();

    expect(blockProperty("table", "tine.filter")).toBe("points > 2");
    expect(doc.byId.table.raw).toBe("Table\ntine.filter:: points > 2");

    openFormulaEditor({
      mode: "filter",
      ownerId: "table",
      x: 10,
      y: 10,
      expr: "points > 2",
      formulas: [],
      fields: ["points"],
    });
    textarea = root.querySelector(".formula-editor-textarea") as HTMLTextAreaElement;
    textarea.value = "";
    input(textarea);
    saveButton(root).click();

    expect(blockProperty("table", "tine.filter")).toBeNull();
    expect(doc.byId.table.raw).toBe("Table");
    dispose();
  });
});
