import { afterEach, describe, expect, it } from "vitest";
import {
  activeSurface,
  clearFocusSurface,
  editingId,
  editingOwner,
  endEdit,
  focusSurfaceFor,
  noteSurfaceFocused,
  startEditing,
  takeCaretFor,
} from "./editorController";

describe("editorController", () => {
  afterEach(() => {
    endEdit("blur");
    clearFocusSurface("block-a");
    clearFocusSurface("block-b");
    clearFocusSurface("block-c");
  });

  it("starts editing with owner and a one-shot caret target", () => {
    startEditing("block-a", 4, "owner-a");

    expect(editingId()).toBe("block-a");
    expect(editingOwner()).toBe("owner-a");
    expect(takeCaretFor("block-a")).toBe(4);
    expect(takeCaretFor("block-a")).toBeNull();
  });

  it("ends editing by clearing both editing signals", () => {
    startEditing("block-b", 0, "owner-b");

    endEdit("blur");

    expect(editingId()).toBeNull();
    expect(editingOwner()).toBeNull();
  });

  it("stamps unscoped starts and clears stamps for scoped starts", () => {
    noteSurfaceFocused("surface-a");

    startEditing("block-c", 0, null);
    expect(activeSurface()).toBe("surface-a");
    expect(focusSurfaceFor("block-c")).toBe("surface-a");

    startEditing("block-c", 0, "owner-c");
    expect(focusSurfaceFor("block-c")).toBeUndefined();
  });
});
