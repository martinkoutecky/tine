import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { endEdit } from "../editorController";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { bumpGraphEpoch } from "../graphSession";
import { handleCellSelectionKey, resetCellSelectionForTests, setCellSel } from "./selection";

function node(id: string, raw: string, parent: string | null, children: string[] = []) {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

describe("typing over a selected sheet cell (I-20)", () => {
  let textarea: HTMLTextAreaElement;
  beforeEach(() => {
    setDoc({
      byId: {
        grid: node("grid", "Grid\ntine.view:: grid", null, ["r1"]),
        r1: node("r1", "", "grid", ["c1"]),
        c1: node("c1", "A", "r1"),
      },
      pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["grid"], format: "md", readOnly: false, guide: false }],
      feed: ["Sheet"],
      loaded: true,
    });
    const cell = document.createElement("div");
    cell.className = "sheet-cell";
    cell.dataset.sheetGridId = "grid";
    cell.dataset.row = "0";
    cell.dataset.col = "0";
    textarea = document.createElement("textarea");
    textarea.className = "block-editor";
    textarea.value = "A";
    cell.append(textarea);
    document.body.append(cell);
    setCellSel({ gridId: "grid", row: 0, col: 0 });
  });
  afterEach(() => {
    document.body.innerHTML = "";
    resetCellSelectionForTests();
    resetStore();
    endEdit("blur");
  });
  const type = (key: string) => handleCellSelectionKey({
    key, code: "", shiftKey: false, ctrlKey: false, metaKey: false, altKey: false, isComposing: false,
    preventDefault() {}, stopPropagation() {},
  } as unknown as KeyboardEvent);
  const flush = () => new Promise((resolve) => setTimeout(resolve, 30));

  it("replaces the cell text through the editor it started", async () => {
    expect(type("x")).toBe(true);
    await flush();
    expect(textarea.value).toBe("x");
  });
  it("does not type into a same-coordinate cell after another graph opened", async () => {
    expect(type("x")).toBe(true);
    bumpGraphEpoch();
    await flush();
    expect(textarea.value).toBe("A");
  });
  it("does not type once the editor the keystroke started was closed", async () => {
    expect(type("x")).toBe(true);
    endEdit("blur");
    await flush();
    expect(textarea.value).toBe("A");
  });
});
