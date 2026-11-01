import { describe, expect, it, vi } from "vitest";
import { EditorSelection, EditorState } from "@codemirror/state";
import { Direction, type EditorView } from "@codemirror/view";
import { cm6ArrowLeave } from "./cm6ArrowLeave";

const viewAt = (head: number, assoc = 0) => ({
  state: EditorState.create({ doc: "English", selection: EditorSelection.create([EditorSelection.cursor(head, assoc)]) }),
  textDirectionAt: vi.fn(() => Direction.LTR),
  moveByChar: vi.fn((cursor) => cursor),
});
describe("CM6 visual block exits", () => {
  it("does no direction or layout reads for interior and non-horizontal keys", () => {
    const v = viewAt(3);
    expect(cm6ArrowLeave("ArrowRight", v as unknown as EditorView)).toBeNull();
    expect(cm6ArrowLeave("ArrowUp", v as unknown as EditorView)).toBeNull();
    expect(v.textDirectionAt).not.toHaveBeenCalled();
    expect(v.moveByChar).not.toHaveBeenCalled();
  });
  it("passes the actual range's association through and lets an affinity-only move happen", () => {
    const v = viewAt(0, 1);
    v.moveByChar.mockImplementation(() => EditorSelection.cursor(0, -1));
    expect(cm6ArrowLeave("ArrowLeft", v as unknown as EditorView)).toBeNull();
    expect(v.moveByChar).toHaveBeenCalledWith(v.state.selection.main, false);
  });
  it("keeps ordinary LTR exits and defers actual visual movement", () => {
    const v = viewAt(0, -1);
    expect(cm6ArrowLeave("ArrowLeft", v as unknown as EditorView)).toBe("backward");
    v.moveByChar.mockImplementation(() => EditorSelection.cursor(1));
    expect(cm6ArrowLeave("ArrowRight", v as unknown as EditorView)).toBeNull();
    const end = viewAt(7, 1);
    expect(cm6ArrowLeave("ArrowRight", end as unknown as EditorView)).toBe("forward");
  });
});
