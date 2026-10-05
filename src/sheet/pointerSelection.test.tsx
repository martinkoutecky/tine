import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bumpGraphEpoch } from "../graphSession";

const selection = vi.hoisted(() => ({
  extendCellSelectionTo: vi.fn(),
  setCellRangeSelection: vi.fn(),
  setCellSel: vi.fn(),
}));
vi.mock("./selection", () => selection);
import { beginCellPointerSelection } from "./pointerSelection";

function cell(row: number, col: number): HTMLElement {
  const el = document.createElement("div");
  el.className = "sheet-cell";
  el.dataset.sheetGridId = "g";
  el.dataset.row = String(row);
  el.dataset.col = String(col);
  document.body.append(el);
  return el;
}
const pointer = (type: string, init: Partial<PointerEventInit> & { clientX?: number } = {}) =>
  new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init });

describe("sheet drag selection owns its window listeners (I-20/I-21)", () => {
  let a: HTMLElement;
  let b: HTMLElement;
  beforeEach(() => {
    document.body.innerHTML = "";
    a = cell(0, 0);
    b = cell(0, 1);
    document.elementFromPoint = vi.fn(() => b);
    Object.values(selection).forEach((fn) => fn.mockClear());
  });
  afterEach(() => { document.body.innerHTML = ""; });

  const begin = () => {
    const down = pointer("pointerdown");
    Object.defineProperty(down, "target", { value: a });
    expect(beginCellPointerSelection(down, "g")).toBe(true);
  };
  const drag = () => window.dispatchEvent(pointer("pointermove", { clientX: 40 }));

  it("extends the range while the same pointer drags", () => {
    begin();
    drag();
    expect(selection.setCellRangeSelection).toHaveBeenCalledOnce();
  });
  it("stops after the pointer is released", () => {
    begin();
    window.dispatchEvent(pointer("pointerup"));
    drag();
    expect(selection.setCellRangeSelection).not.toHaveBeenCalled();
  });
  it("ignores another pointer", () => {
    begin();
    window.dispatchEvent(pointer("pointermove", { pointerId: 2, clientX: 40 }));
    expect(selection.setCellRangeSelection).not.toHaveBeenCalled();
    drag();
    expect(selection.setCellRangeSelection).toHaveBeenCalledOnce();
  });
  it("ends when the window loses focus without a release", () => {
    begin();
    window.dispatchEvent(new Event("blur"));
    drag();
    expect(selection.setCellRangeSelection).not.toHaveBeenCalled();
  });
  it("ends when another graph opens before any release", () => {
    begin();
    bumpGraphEpoch();
    drag();
    expect(selection.setCellRangeSelection).not.toHaveBeenCalled();
  });
  it("ends when the anchor cell is removed before any release", () => {
    begin();
    a.remove();
    drag();
    expect(selection.setCellRangeSelection).not.toHaveBeenCalled();
  });
});
