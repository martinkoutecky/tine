import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { batch, createComputed, createRoot } from "solid-js";
import { loadavg } from "node:os";
import { writeFileSync } from "node:fs";
import { setFlagsFromString } from "node:v8";
import { runInNewContext } from "node:vm";

const cost = vi.hoisted(() => ({ reads: 0, children: 0, arrays: 0, counting: false, visited: new Set<string>(), order: null as WeakRef<string[]> | null }));
vi.mock("./document/model", async original => {
  const model = await original<typeof import("./document/model")>();
  const byId = new Proxy({}, {
    get(_target, key) {
      cost.reads++;
      if (cost.counting && typeof key === "string") cost.visited.add(key);
      return Reflect.get(model.doc.byId, key);
    },
    ownKeys() { return Reflect.ownKeys(model.doc.byId); },
    getOwnPropertyDescriptor(_target, key) { return Reflect.getOwnPropertyDescriptor(model.doc.byId, key); },
  });
  return { ...model, doc: new Proxy(model.doc, {
    get(target, key) { return key === "byId" ? byId : Reflect.get(target, key); },
  }) };
});
vi.mock("./document/commentOrder", async original => {
  const order = await original<typeof import("./document/commentOrder")>();
  return { ...order, commentsFirst: (...args: Parameters<typeof order.commentsFirst>) => {
    cost.children += args[0].length; return order.commentsFirst(...args);
  } };
});
vi.mock("./document/tree", async original => {
  const tree = await original<typeof import("./document/tree")>();
  return { ...tree, scopedVisibleOrder: (...args: Parameters<typeof tree.scopedVisibleOrder>) => {
    const result = tree.scopedVisibleOrder(...args);
    Object.defineProperty(result, "filter", { value: function(this: string[], ...filterArgs: unknown[]) {
      const filtered = Reflect.apply(Array.prototype.filter, this, filterArgs);
      cost.order = new WeakRef(filtered); return filtered;
    } });
    return result;
  } };
});
import { initParser } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { clearSelection, extendSelectionTo, isSelected, moveSelection, resetStore, selectBlock, selectedIds } from "./document";
import { doc, setDoc } from "./document/model";
import { ChangeKeyRegistry } from "./changeKeyRegistry";
import type { BlockDto } from "./types";

const block = (id: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({ id, raw: id, children, collapsed });
type Shape = "flat" | "deep" | "folded";
function fixture(size: number, shape: Shape) {
  const roots: BlockDto[] = [];
  for (let at = 0; at < size;) {
    const root = block(`n${at++}`); roots.push(root);
    if (shape === "flat") continue;
    let parent = root;
    const length = shape === "deep" ? 32 : 10;
    for (let level = 1; level < length && at < size; level++) {
      const child = block(`n${at++}`); parent.children.push(child);
      if (shape === "deep") parent = child;
    }
    root.collapsed = shape === "folded";
  }
  loadSingle({ name: "Cold", title: "Cold", kind: "page", pre_block: null, blocks: roots });
  return roots.length;
}
beforeAll(initParser);
afterEach(resetStore);

const rows: object[] = [];
// Opt-in measurement uses fresh GC turns: WeakRef targets stay alive for the
// entire job in which they were dereferenced. Normal CI pins counts and the two
// release invariants; the full heap measurement table is opt-in.
const measuring = !!process.env.SEL_COST_OUTPUT;
let gc: (() => void) | undefined;
async function heap() {
  if (!gc) {
    setFlagsFromString("--expose_gc");
    gc = runInNewContext("gc") as () => void;
  }
  await new Promise<void>(resolve => setImmediate(resolve));
  gc(); gc();
  return process.memoryUsage().heapUsed;
}

it.each([1000, 10000, 50000].flatMap(size => (["flat", "deep", "folded"] as Shape[]).map(shape => ({ size, shape }))))(
  "cold selection costs visible rows: $size $shape", async ({ size, shape }) => {
    const roots = fixture(size, shape);
    const scope = { roots: [...doc.pages[0].roots] };
    const before = measuring ? await heap() : 0;
    cost.reads = 0; cost.children = 0; cost.arrays = 0;
    cost.visited.clear(); cost.counting = true;
    // Count returned arrays, including intermediate mapped tuples' outer array;
    // literal arrays are separately audited in the receipt.
    const methods = ["map", "filter", "slice"] as const;
    const spies = methods.map(method => {
      const original = Array.prototype[method];
      return vi.spyOn(Array.prototype, method).mockImplementation(function(this: unknown[], ...args: unknown[]) {
        cost.arrays++; return Reflect.apply(original, this, args);
      });
    });
    const started = performance.now();
    selectBlock("n0", scope);
    const ms = performance.now() - started;
    const counts = { nodesVisited: cost.visited.size, reads: cost.reads, childEntries: cost.children, methodArrays: cost.arrays };
    cost.counting = false; cost.visited.clear();
    for (const spy of spies) spy.mockRestore();
    const retained = measuring ? await heap() - before : 0;
    const visible = shape === "folded" ? roots : size;
    clearSelection();
    const cleared = measuring ? await heap() : 0;
    const released = measuring ? cost.order?.deref() === undefined : null;
    // A second cold topology isolates its retained heap from the document's
    // first-read Solid property signals, which live until the page is dropped.
    selectBlock("n0", scope);
    const topologyBytes = measuring ? await heap() - cleared : 0;
    // Bulk enumeration remains complete even when cold topology skips children.
    expect(isSelected(shape === "flat" ? "n0" : "n1")).toBe(true);
    expect(selectedIds()).toHaveLength(shape === "flat" ? 1 : shape === "deep" ? 32 : 10);
    clearSelection();
    resetStore();
    const dropped = measuring ? await heap() : 0;
    rows.push({ size, shape, visible, ...counts, retained, topologyBytes, clearedDelta: cleared - before, droppedDelta: dropped - before, released, ms, load: loadavg() });
    if (measuring) writeFileSync(process.env.SEL_COST_OUTPUT!, JSON.stringify(rows, null, 2));
    expect(counts.reads).toBeLessThanOrEqual(visible * (shape === "deep" ? 60 : 9) + 64);
    expect(counts.nodesVisited).toBeLessThanOrEqual(visible + 32);
    expect(counts.childEntries).toBeLessThanOrEqual(1);
    expect(counts.methodArrays).toBeLessThanOrEqual(3);
    if (measuring) expect(released).toBe(true);
  },
);

it.each([1000, 10000])("query notifies only changed membership among %i mounted readers", size => {
  fixture(size, "flat");
  const scope = { roots: [...doc.pages[0].roots], navOnly: true };
  selectBlock("n0", scope);
  extendSelectionTo(`n${size / 2}`, scope);
  let renders = 0;
  const dispose = createRoot(dispose => {
    for (let at = 0; at < size; at++) createComputed(() => { isSelected(`n${at}`); renders++; });
    return dispose;
  });
  const advance = vi.spyOn(ChangeKeyRegistry.prototype, "advance");
  renders = 0; cost.children = 0; cost.reads = 0;
  for (let at = 0; at < 10; at++) moveSelection(1, true);
  expect(renders).toBe(10);
  expect(advance).toHaveBeenCalledTimes(10);
  expect(cost.children).toBeLessThanOrEqual(120);
  expect(cost.reads).toBeLessThanOrEqual(500);
  if (measuring) {
    rows.push({ queryReaders: size, initialSelected: size / 2 + 1, keys: 10, notifications: advance.mock.calls.length, renders, reads: cost.reads, childEntries: cost.children });
    writeFileSync(process.env.SEL_COST_OUTPUT!, JSON.stringify(rows, null, 2));
  }
  advance.mockRestore(); dispose();
});

it("with no selection, mounted rows answer membership without reading ancestry", () => {
  fixture(10000, "deep");
  cost.reads = 0;
  const dispose = createRoot(dispose => {
    for (let at = 0; at < 10000; at++) createComputed(() => { isSelected(`n${at}`); });
    return dispose;
  });
  expect(cost.reads).toBe(0);
  dispose();
});

it("outline replacement releases the old visible order", async () => {
  fixture(1000, "folded");
  selectBlock("n0", { roots: [...doc.pages[0].roots] });
  moveSelection(1, true);
  const previous = cost.order;
  setDoc("byId", "n0", "collapsed", false);
  extendSelectionTo("n10");
  await heap(); expect(previous?.deref()).toBeUndefined();
  expect(selectedIds()).toHaveLength(20);
});

it("mounted hidden membership follows reparenting and removal", () => {
  fixture(1000, "folded");
  selectBlock("n0", { roots: [...doc.pages[0].roots] });
  const values: boolean[] = [];
  const dispose = createRoot(dispose => {
    createComputed(() => { values.push(isSelected("n1")); });
    return dispose;
  });
  const reparent = (from: string, to: string) => batch(() => {
    setDoc("byId", from, "children", ids => ids.filter(id => id !== "n1"));
    setDoc("byId", to, "children", ids => [...ids, "n1"]);
    setDoc("byId", "n1", "parent", to);
  });
  reparent("n0", "n10");
  expect(values.at(-1)).toBe(false);
  reparent("n10", "n0");
  expect(values.at(-1)).toBe(true);
  batch(() => {
    setDoc("byId", "n0", "children", ids => ids.filter(id => id !== "n1"));
    setDoc("byId", "n1", undefined!);
  });
  expect(values.at(-1)).toBe(false);
  dispose();
});

it("materialized IDs follow structural edits below a folded descendant", () => {
  fixture(1000, "folded");
  selectBlock("n0", { roots: [...doc.pages[0].roots] });
  expect(selectedIds()).toHaveLength(10);
  setDoc("byId", "added", { id: "added", raw: "added", collapsed: false, parent: "n1", page: "Cold", children: [] });
  setDoc("byId", "n1", "children", ["added"]);
  expect(selectedIds()).toContain("added");
  setDoc("byId", "n1", "children", []);
  expect(selectedIds()).not.toContain("added");
});

it("clearing a materialized full-page selection releases its topology", async () => {
  fixture(10000, "folded");
  selectBlock("n0", { roots: [...doc.pages[0].roots] });
  extendSelectionTo("n9990");
  expect(selectedIds()).toHaveLength(10000);
  const previous = cost.order;
  clearSelection();
  await heap(); expect(previous?.deref()).toBeUndefined();
  expect(selectedIds()).toEqual([]);
});
