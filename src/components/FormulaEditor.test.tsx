import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { FormulaEditor } from "./FormulaEditor";
import { initParser } from "../render/parse";
import { blockProperty, doc, resetStore, setDoc, type FeedPage, type Node as StoreNode } from "../store";
import { closeFormulaEditor, openFormulaEditor } from "../ui";
import { decodeFormulaExpr, encodeFormulaExpr } from "../sheet/formula";
import {
  clearTransientLayersForTest,
  dismissTopTransient,
  registerTransientLayer,
  topTransientLayer,
  type TransientDismissReason,
} from "../transientLayers";

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  closeFormulaEditor();
  resetStore();
  document.body.innerHTML = "";
  clearTransientLayersForTest();
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

function change(target: EventTarget): Event {
  const event = new Event("change", { bubbles: true, cancelable: true });
  target.dispatchEvent(event);
  return event;
}

function saveButton(root: ParentNode): HTMLButtonElement {
  const button = [...root.querySelectorAll(".formula-editor-btn")]
    .find((el) => el.textContent?.trim() === "Save") as HTMLButtonElement | undefined;
  if (!button) throw new Error("missing Save button");
  return button;
}

function rawToggle(root: ParentNode): HTMLButtonElement {
  const button = root.querySelector(".formula-editor-raw-toggle") as HTMLButtonElement | null;
  if (!button) throw new Error("missing raw toggle");
  return button;
}

function valueFaces(root: ParentNode): HTMLButtonElement[] {
  return [...root.querySelectorAll<HTMLButtonElement>(".formula-builder-value-face")];
}

function pickerFor(face: HTMLButtonElement): HTMLDivElement | null {
  return face.closest(".formula-builder-value-wrap")?.querySelector<HTMLDivElement>(".formula-builder-picker") ?? null;
}

function openValueEditor(expr = '"todo"') {
  loadEditorDoc();
  const mounted = mount(() => <FormulaEditor />);
  openFormulaEditor({
    mode: "edit",
    ownerId: "table",
    x: 10,
    y: 10,
    name: "label",
    expr,
    formulas: [],
    fields: ["status", "points"],
    home: { kind: "block", id: "table" },
  });
  return mounted;
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
    rawToggle(root).click();
    textarea = root.querySelector(".formula-editor-textarea") as HTMLTextAreaElement;
    textarea.value = "";
    input(textarea);
    saveButton(root).click();

    expect(blockProperty("table", "tine.filter")).toBeNull();
    expect(doc.byId.table.raw).toBe("Table");
    dispose();
  });

  it("opens a conditional formula as an IF/THEN/ELSE face", () => {
    loadEditorDoc();
    const { root, dispose } = mount(() => <FormulaEditor />);
    openFormulaEditor({
      mode: "edit",
      ownerId: "table",
      x: 10,
      y: 10,
      name: "next",
      expr: 'if(isEmpty(status), "todo", status)',
      formulas: [],
      fields: ["status"],
      home: { kind: "block", id: "table" },
    });

    const face = root.querySelector(".formula-builder-if");
    expect(face).not.toBeNull();
    expect(face?.textContent).toContain("IF");
    expect(face?.textContent).toContain("THEN");
    expect(face?.textContent).toContain("ELSE");
    expect(root.querySelector(".formula-editor-textarea")).toBeNull();
    dispose();
  });

  it("changing the operator in a comparison rewrites the expression text", () => {
    loadEditorDoc();
    const { root, dispose } = mount(() => <FormulaEditor />);
    openFormulaEditor({
      mode: "filter",
      ownerId: "table",
      x: 10,
      y: 10,
      expr: "points > 2",
      formulas: [],
      fields: ["points"],
    });

    const op = root.querySelector(".formula-builder-operator") as HTMLSelectElement;
    op.value = "<=";
    change(op);
    rawToggle(root).click();

    const textarea = root.querySelector(".formula-editor-textarea") as HTMLTextAreaElement;
    expect(textarea.value).toBe("points <= 2");
    dispose();
  });

  it("selecting a field in a value face rewrites the expression text", () => {
    loadEditorDoc();
    const { root, dispose } = mount(() => <FormulaEditor />);
    openFormulaEditor({
      mode: "edit",
      ownerId: "table",
      x: 10,
      y: 10,
      name: "label",
      expr: '"todo"',
      formulas: [],
      fields: ["status", "points"],
      home: { kind: "block", id: "table" },
    });

    (root.querySelector(".formula-builder-value-face") as HTMLButtonElement).click();
    const status = [...root.querySelectorAll(".formula-builder-pick-field")]
      .find((el) => el.textContent?.trim() === "status") as HTMLButtonElement;
    status.click();
    rawToggle(root).click();

    const textarea = root.querySelector(".formula-editor-textarea") as HTMLTextAreaElement;
    expect(textarea.value).toBe("status");
    dispose();
  });

  it("preserves an unrepresentable expression verbatim through the raw-expression face on save", () => {
    loadEditorDoc();
    const { root, dispose } = mount(() => <FormulaEditor />);
    const original = "((price + qty) * (tax - discount)) / count";
    openFormulaEditor({
      mode: "add",
      ownerId: "table",
      x: 10,
      y: 10,
      expr: original,
      formulas: [],
      fields: ["price", "qty", "tax", "discount", "count"],
    });

    const name = root.querySelector(".formula-editor-input") as HTMLInputElement;
    name.value = "complex";
    input(name);
    const raw = root.querySelector(".formula-builder-root-raw input") as HTMLInputElement;
    expect(raw).not.toBeNull();
    expect(raw.value).toBe(original);
    saveButton(root).click();

    expect(decodeFormulaExpr(blockProperty("table", "tine.formula.complex") ?? "")).toBe(original);
    dispose();
  });

  it("the raw toggle round-trips builder to textarea and back without changing the expression", () => {
    loadEditorDoc();
    const { root, dispose } = mount(() => <FormulaEditor />);
    openFormulaEditor({
      mode: "filter",
      ownerId: "table",
      x: 10,
      y: 10,
      expr: "points > 2",
      formulas: [],
      fields: ["points"],
    });

    rawToggle(root).click();
    let textarea = root.querySelector(".formula-editor-textarea") as HTMLTextAreaElement;
    expect(textarea.value).toBe("points > 2");
    rawToggle(root).click();
    expect(root.querySelector(".formula-builder-operator")).not.toBeNull();
    rawToggle(root).click();
    textarea = root.querySelector(".formula-editor-textarea") as HTMLTextAreaElement;
    expect(textarea.value).toBe("points > 2");
    dispose();
  });

  for (const reason of ["escape", "back"] satisfies TransientDismissReason[]) {
    it(`closes a value picker before the formula editor on registry ${reason}`, async () => {
      const lower = vi.fn(() => true);
      const unregisterLower = registerTransientLayer({ id: `formula-picker-${reason}-lower`, dismiss: lower });
      const { root, dispose } = openValueEditor();
      const face = valueFaces(root)[0];
      face.click();
      expect(pickerFor(face)).not.toBeNull();

      expect(dismissTopTransient(reason)).toBe(true);
      await Promise.resolve();
      expect(pickerFor(face)).toBeNull();
      expect(root.querySelector(".formula-editor")).not.toBeNull();
      expect(lower).not.toHaveBeenCalled();
      expect(document.activeElement).toBe(face);

      expect(dismissTopTransient(reason)).toBe(true);
      await Promise.resolve();
      expect(root.querySelector(".formula-editor")).toBeNull();
      expect(lower).not.toHaveBeenCalled();

      expect(dismissTopTransient(reason)).toBe(true);
      expect(lower).toHaveBeenCalledOnce();
      unregisterLower();
      dispose();
    });
  }

  it("keeps multiple value pickers instance-safe and reactivates an older visible peer", async () => {
    const { root, dispose } = openValueEditor("points > 2");
    const [left, right] = valueFaces(root);
    left.click();
    const leftId = topTransientLayer()?.id;
    right.click();
    const rightId = topTransientLayer()?.id;

    expect(leftId).toMatch(/^formula-picker-/);
    expect(rightId).toMatch(/^formula-picker-/);
    expect(rightId).not.toBe(leftId);
    expect(pickerFor(left)).not.toBeNull();
    expect(pickerFor(right)).not.toBeNull();

    pickerFor(left)?.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    expect(topTransientLayer()?.id).toBe(leftId);
    expect(dismissTopTransient("escape")).toBe(true);
    await Promise.resolve();
    expect(pickerFor(left)).toBeNull();
    expect(pickerFor(right)).not.toBeNull();

    expect(dismissTopTransient("back")).toBe(true);
    await Promise.resolve();
    expect(pickerFor(right)).toBeNull();
    expect(root.querySelector(".formula-editor")).not.toBeNull();
    dispose();
  });

  it("dismisses raw and transform drafts without mutating the expression", async () => {
    const { root, dispose } = openValueEditor();
    let face = valueFaces(root)[0];
    face.click();
    const raw = pickerFor(face)?.querySelector<HTMLInputElement>(".formula-builder-raw-input");
    if (!raw) throw new Error("missing raw expression input");
    raw.focus();
    raw.value = "status";
    input(raw);

    expect(dismissTopTransient("escape")).toBe(true);
    await Promise.resolve();
    expect(pickerFor(face)).toBeNull();
    expect(face.textContent?.trim()).toBe('"todo"');

    face.click();
    const replace = [...(pickerFor(face)?.querySelectorAll<HTMLButtonElement>(".formula-builder-transform") ?? [])]
      .find((button) => button.textContent?.trim() === "replace text");
    if (!replace) throw new Error("missing replace transform");
    replace.click();
    const transformInput = pickerFor(face)?.querySelector<HTMLInputElement>(".formula-builder-transform-args .qb-input");
    if (!transformInput) throw new Error("missing transform input");
    transformInput.focus();
    transformInput.value = "draft needle";
    input(transformInput);

    expect(dismissTopTransient("back")).toBe(true);
    await Promise.resolve();
    face = valueFaces(root)[0];
    expect(pickerFor(face)).toBeNull();
    expect(face.textContent?.trim()).toBe('"todo"');
    rawToggle(root).click();
    expect((root.querySelector(".formula-editor-textarea") as HTMLTextAreaElement).value).toBe('"todo"');
    dispose();
  });

  it("unregisters a picker after explicit commit and after component disposal", () => {
    const lower = vi.fn(() => true);
    const unregisterLower = registerTransientLayer({ id: "formula-picker-cleanup-lower", dismiss: lower });
    const { root, dispose } = openValueEditor();
    const face = valueFaces(root)[0];
    face.click();
    const status = [...(pickerFor(face)?.querySelectorAll<HTMLButtonElement>(".formula-builder-pick-field") ?? [])]
      .find((button) => button.textContent?.trim() === "status");
    if (!status) throw new Error("missing status field");
    status.click();
    expect(pickerFor(valueFaces(root)[0])).toBeNull();
    expect(dismissTopTransient("escape")).toBe(true);
    expect(root.querySelector(".formula-editor")).toBeNull();
    expect(lower).not.toHaveBeenCalled();

    openFormulaEditor({
      mode: "edit",
      ownerId: "table",
      x: 10,
      y: 10,
      name: "label",
      expr: '"todo"',
      formulas: [],
      fields: ["status"],
      home: { kind: "block", id: "table" },
    });
    valueFaces(root)[0].click();
    dispose();
    expect(dismissTopTransient("back")).toBe(true);
    expect(lower).toHaveBeenCalledOnce();
    unregisterLower();
  });
});
