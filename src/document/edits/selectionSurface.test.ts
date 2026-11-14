import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createComputed, createRoot, createSignal } from "solid-js";
import { backend } from "../../backend";
import { loadFeed, resetStore, isDirty, isSelected, selectBlock, selectedIds } from "..";
import { ownSelectionSurface } from "./selection";
import { registerOutlineSelectionListener } from "../../modeHooks";

const disposers: (() => void)[] = [];
beforeEach(() => loadFeed([{ name: "Surface", title: "Surface", kind: "page", pre_block: null,
  blocks: Array.from({ length: 33 }, (_, i) => ({ id: `b${i}`, raw: `Block ${i}`, children: [], collapsed: false })) }]));
afterEach(() => {
  for (const dispose of disposers.splice(0).reverse()) dispose();
  resetStore(); vi.restoreAllMocks();
});

it.each([1, 32])("route retirement is synchronous, reads its route once, and notifies only selected readers among %i others", size => {
  const [route, setRoute] = createSignal("A");
  let reads = 0, selectedUpdates = 0, otherUpdates = 0;
  const read = vi.spyOn(backend(), "getPage");
  const submit = vi.spyOn(backend(), "pageSubmit");
  disposers.push(createRoot(dispose => {
    ownSelectionSurface("main", () => { reads++; return route(); });
    createComputed(() => { isSelected("b0"); selectedUpdates++; });
    for (let i = 1; i <= size; i++) createComputed(() => { isSelected(`b${i}`); otherUpdates++; });
    return dispose;
  }));
  selectBlock("b0", null, "main");
  selectedUpdates = otherUpdates = reads = 0;
  setRoute("B");
  expect(selectedIds()).toEqual([]);
  expect({ reads, selectedUpdates, otherUpdates }).toEqual({ reads: 1, selectedUpdates: 1, otherUpdates: 0 });
  expect(read).not.toHaveBeenCalled(); expect(submit).not.toHaveBeenCalled();
  expect(isDirty("Surface")).toBe(false);
});

it("an old occurrence's cleanup cannot clear selection in its replacement with the same surface key", () => {
  const first = createRoot(dispose => { ownSelectionSurface("same"); return dispose; });
  const second = createRoot(dispose => { ownSelectionSurface("same"); return dispose; });
  disposers.push(first, second);
  selectBlock("b0", null, "same");
  first();
  expect(selectedIds()).toEqual(["b0"]);
  second();
  expect(selectedIds()).toEqual([]);
});

it("a parent route retires its nested reference/cell selection but another surface cannot", () => {
  const [route, setRoute] = createSignal("A");
  const other = createRoot(dispose => { ownSelectionSurface("other"); return dispose; });
  disposers.push(other, createRoot(dispose => {
    ownSelectionSurface("main", route);
    ownSelectionSurface("ref:query", undefined, "main");
    ownSelectionSurface("sheet:cell", undefined, "ref:query");
    return dispose;
  }));
  selectBlock("b0", null, "sheet:cell");
  other();
  expect(selectedIds()).toEqual(["b0"]);
  setRoute("B");
  expect(selectedIds()).toEqual([]);
});

it("a surface retired while ending an edit cannot start a hidden selection", () => {
  const dispose = createRoot(dispose => { ownSelectionSurface("departing"); return dispose; });
  disposers.push(dispose, registerOutlineSelectionListener(dispose));
  selectBlock("b0", null, "departing");
  expect(selectedIds()).toEqual([]);
});

it("route replacement retains one token per live key and unmount releases the registry entry", () => {
  const key = "selection-lifetime-bound";
  let registry: Map<unknown, unknown> | undefined;
  const original = Map.prototype.set;
  vi.spyOn(Map.prototype, "set").mockImplementation(function(this: Map<unknown, unknown>, at, value) {
    if (at === key) registry = this;
    return original.call(this, at, value);
  });
  const [route, setRoute] = createSignal(0);
  const dispose = createRoot(dispose => { ownSelectionSurface(key, route); return dispose; });
  disposers.push(dispose);
  const liveSize = registry!.size;
  expect(registry!.has(key)).toBe(true);
  for (let i = 1; i <= 32; i++) setRoute(i);
  expect(registry!.size).toBe(liveSize);
  dispose();
  expect(registry!.has(key)).toBe(false);
  expect(registry!.size).toBe(liveSize - 1);
});
