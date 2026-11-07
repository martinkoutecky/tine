import { afterEach, describe, expect, it } from "vitest";
import { createRoot, createComputed } from "solid-js";
import * as controller from "./editorController";
import {
  activeSurface,
  clearFocusSurface,
  editingId,
  editingOwner,
  endEdit,
  endEditForSurface,
  focusSurfaceFor,
  noteSurfaceFocused,
  startEditing,
  takeCaretFor,
} from "./editorController";

describe("editorController", () => {
  it.each([8, 32])("moving the editor does not revisit %i unrelated block predicates", size => {
    // The pre-keyed public boundary was the editingId equality predicate.
    const matches = (controller as Partial<{ isEditingId: (id: string) => boolean }>).isEditingId
      ?? ((id: string) => editingId() === id);
    let unrelated = 0, relevant = 0;
    const dispose = createRoot(dispose => {
      for (let i = 0; i < size; i++) createComputed(() => { matches(`unrelated${i}`); unrelated++; });
      for (const id of ["block-a", "block-b"]) createComputed(() => { matches(id); relevant++; });
      return dispose;
    });
    try {
      unrelated = relevant = 0;
      startEditing("block-a", 0, "owner-a");
      expect(unrelated).toBe(0);
      expect(relevant).toBe(1);
      unrelated = relevant = 0;
      startEditing("block-b", 0, "owner-b");
      expect(unrelated).toBe(0);
      expect(relevant).toBe(2);
    } finally { dispose(); }
  });
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

  it("only ends editing for navigation in the active surface", () => {
    noteSurfaceFocused("pane:a");
    startEditing("block-a", 0, null);

    endEditForSurface("page-navigation", "pane:b");
    expect(editingId()).toBe("block-a");

    endEditForSurface("page-navigation", "pane:a");
    expect(editingId()).toBeNull();
  });
});
