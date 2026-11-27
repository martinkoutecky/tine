// GH #874 count gate: deciding whether Backspace at the start of a grid cell's first child, or
// Delete at the end of the cell's text, removes that child reads the cell block and the child
// only, however many children the cell has, and asks no caret navigation (`gridNeighbour`,
// `gridEntry`) or visible-order walk (`bodyNeighbor`).
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const probe = vi.hoisted(() => ({ visited: new Set<string>(), walks: 0 }));

vi.mock("../document", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../document")>();
  return {
    ...actual,
    node: ((id: string) => { probe.visited.add(id); return actual.node(id); }) as typeof actual.node,
    bodyNeighbor: ((...args: Parameters<typeof actual.bodyNeighbor>) => { probe.walks++; return actual.bodyNeighbor(...args); }) as typeof actual.bodyNeighbor,
  };
});
vi.mock("./gridNeighbour", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./gridNeighbour")>();
  const counted = <A extends unknown[], R>(f: (...args: A) => R) => (...args: A): R => { probe.walks++; return f(...args); };
  // `firstInside` and `edgeCell` are reached only through these two.
  return { ...actual, gridNeighbour: counted(actual.gridNeighbour), gridEntry: counted(actual.gridEntry) };
});

import { initParser } from "../render/parse";
import { isDirty, resetStore } from "../document";
import { doc, setDoc, type Node as StoreNode } from "../document/model";
import { gridEditKey } from "./gridEditMove";

beforeAll(async () => {
  await initParser();
});
beforeEach(() => resetStore());

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

/** A grid cell `c1` whose first child `k` holds `kRaw`, followed by `n` ordinary children. */
function load(kRaw: string, n: number) {
  const later = Array.from({ length: n }, (_, i) => `b${i}`);
  const byId: Record<string, StoreNode> = {
    grid: node("grid", "Grid\ntine.view:: grid", null, ["r1"]),
    r1: node("r1", "", "grid", ["c1"]),
    c1: node("c1", "Cell", "r1", ["k", ...later]),
    k: node("k", kRaw, "c1"),
  };
  for (const id of later) byId[id] = node(id, `Block ${id}`, "c1");
  setDoc({
    byId,
    pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["grid"], format: "md", readOnly: false, guide: false }],
    feed: ["Sheet"],
    loaded: true,
  });
}

function press(key: "Backspace" | "Delete"): { handled: boolean; outline: string[] } {
  probe.visited.clear();
  probe.walks = 0;
  const event = { key, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false } as KeyboardEvent;
  const [id, raw, at] = key === "Delete" ? ["c1", "Cell", 4] : ["k", doc.byId.k.raw, 0];
  const handled = gridEditKey(event, {
    id, cell: { gridId: "grid", row: 0, col: 0 }, raw, start: at, end: at,
    commit: () => {}, softNewline: () => {}, edge: null, col: 0,
  });
  // Blocks of the cell's outline the decision read (the cell block and its grid are not counted).
  return { handled, outline: [...probe.visited].filter(id => id === "k" || /^b\d+$/.test(id)) };
}

describe.each(["Backspace", "Delete"] as const)("GH874 count gate: %s", (key) => {
  it.each([100, 1_000, 10_000])("a refusal (non-empty first child, %i blocks after it) reads the first child only", (n) => {
    load("Child", n);
    const before = JSON.stringify(doc.byId.c1);
    const { handled, outline } = press(key);
    expect(handled).toBe(true);
    expect(probe.walks).toBe(0);
    expect(outline).toEqual(["k"]);
    expect(JSON.stringify(doc.byId.c1)).toBe(before);
    expect(isDirty("Sheet")).toBe(false);
  });

  it.each([100, 1_000, 10_000])("a removal (empty first child, %i blocks after it) decides on the first child only", (n) => {
    load("", n);
    probe.visited.clear();
    const before = doc.byId.c1.children.length;
    const { handled } = press(key);
    expect(handled).toBe(true);
    expect(probe.walks).toBe(0);
    expect(doc.byId.k).toBeUndefined();
    expect(doc.byId.c1.children.length).toBe(before - 1);
    expect(doc.byId.c1.raw).toBe("Cell");
  });
});
