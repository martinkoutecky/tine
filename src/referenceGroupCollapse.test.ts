import { afterEach, expect, it } from "vitest";
import { createComputed, createRoot } from "solid-js";
import { createReferenceGroupCollapse } from "./referenceGroupCollapse";
import { collapsedGroupsFor, resetReferenceSectionState } from "./referenceSectionState";
import type { RefGroup } from "./types";

const group = (page: string): RefGroup => ({ page, kind: "page", blocks: [] });
afterEach(resetReferenceSectionState);

it.each([8, 32])("folding one group does not revisit %i unrelated groups", size => {
  let unrelated = 0, own = 0;
  const dispose = createRoot(dispose => {
    const folds = createReferenceGroupCollapse("linked", () => "Target");
    for (let i = 0; i < size; i++) createComputed(() => { folds.groupCollapsed(group(`Other${i}`)); unrelated++; });
    createComputed(() => { folds.groupCollapsed(group("Café")); own++; });
    unrelated = own = 0;
    folds.setGroupCollapsed(group("CAFE\u0301"), true);
    expect(unrelated).toBe(0);
    expect(own).toBe(1);
    expect(folds.groupCollapsed(group("Café"))).toBe(true);
    const saved = collapsedGroupsFor("linked", "Target");
    folds.setGroupCollapsed(group("Café"), false);
    expect(saved.has("café")).toBe(true);
    return dispose;
  });
  dispose();
});

it("keeps panel snapshots separate until reload and replaces hidden choices on bulk changes", () => {
  createRoot(dispose => {
    const first = createReferenceGroupCollapse("linked", () => "Target");
    first.setGroupCollapsed(group("Hidden"), true);
    const second = createReferenceGroupCollapse("linked", () => "Target");
    first.setGroupCollapsed(group("Shown"), true);
    expect(second.groupCollapsed(group("Shown"))).toBe(false);
    second.reload();
    expect(second.groupCollapsed(group("Shown"))).toBe(true);
    second.setAll([group("Shown")], true);
    expect(second.groupCollapsed(group("Hidden"))).toBe(false);
    first.reload();
    expect(first.groupCollapsed(group("Hidden"))).toBe(false);
    first.setAll([], false);
    expect(collapsedGroupsFor("linked", "Target").size).toBe(0);
    dispose();
  });
});

it("persists the last writer's complete panel snapshot even when another panel wrote meanwhile", () => {
  createRoot(dispose => {
    const first = createReferenceGroupCollapse("linked", () => "Target");
    first.setGroupCollapsed(group("Shared"), true);
    const second = createReferenceGroupCollapse("linked", () => "Target");
    first.setGroupCollapsed(group("First"), true);
    second.setGroupCollapsed(group("Second"), true);
    expect([...collapsedGroupsFor("linked", "Target")]).toEqual(["shared", "second"]);
    first.setGroupCollapsed(group("Third"), true);
    expect([...collapsedGroupsFor("linked", "Target")]).toEqual(["shared", "first", "third"]);
    expect(second.groupCollapsed(group("Third"))).toBe(false);
    dispose();
  });
});
