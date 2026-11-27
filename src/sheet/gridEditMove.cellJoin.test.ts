// GH #874 count gate: Delete at the end of a grid cell's text asks for the block that follows
// the cell's text once, and reads no block of the cell's outline beyond that one, however
// many there are. A refused join writes nothing; the key is the cell's either way, so neither
// editor goes on to its own Delete (whose merge walks the visible order).
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const probe = vi.hoisted(() => ({ visited: new Set<string>(), firstInside: 0 }));

vi.mock("../document", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../document")>();
  return {
    ...actual,
    node: ((id: string) => { probe.visited.add(id); return actual.node(id); }) as typeof actual.node,
    threadRootOf: ((id: string, format?: Parameters<typeof actual.threadRootOf>[1]) => {
      probe.visited.add(id);
      return actual.threadRootOf(id, format);
    }) as typeof actual.threadRootOf,
  };
});
vi.mock("./gridNeighbour", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./gridNeighbour")>();
  return { ...actual, firstInside: (...args: Parameters<typeof actual.firstInside>) => { probe.firstInside++; return actual.firstInside(...args); } };
});

import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { doc, setDoc, type Node as StoreNode } from "../document/model";
import { gridEditKey } from "./gridEditMove";

beforeAll(async () => {
  await initParser();
});
beforeEach(() => resetStore());

function node(id: string, raw: string, parent: string | null, children: string[] = []): StoreNode {
  return { id, raw, collapsed: false, parent, page: "Sheet", children };
}

/** A grid cell `c1` whose first child is `firstRaw`, followed by `n` ordinary children. */
function load(firstRaw: string, n: number) {
  const later = Array.from({ length: n }, (_, i) => `b${i}`);
  const byId: Record<string, StoreNode> = {
    grid: node("grid", "Grid\ntine.view:: grid", null, ["r1"]),
    r1: node("r1", "", "grid", ["c1"]),
    c1: node("c1", "Cell", "r1", ["first", ...later]),
    first: node("first", firstRaw, "c1"),
  };
  for (const id of later) byId[id] = node(id, `Block ${id}`, "c1");
  setDoc({
    byId,
    pages: [{ name: "Sheet", kind: "page", title: "Sheet", preBlock: null, roots: ["grid"], format: "md", readOnly: false, guide: false }],
    feed: ["Sheet"],
    loaded: true,
  });
}

function pressDelete(): { handled: boolean; commits: number; outline: string[] } {
  probe.visited.clear();
  probe.firstInside = 0;
  let commits = 0;
  const key = { key: "Delete", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false } as KeyboardEvent;
  const handled = gridEditKey(key, {
    id: "c1", cell: { gridId: "grid", row: 0, col: 0 }, raw: "Cell", start: 4, end: 4,
    commit: () => { commits++; }, softNewline: () => {}, edge: null, col: 0,
  });
  // Blocks of the cell's outline the gesture read (the cell block and its grid are not counted).
  const outline = [...probe.visited].filter(id => id === "first" || /^b\d+$/.test(id));
  return { handled, commits, outline };
}

describe("GH874 cell join count gate", () => {
  it.each([100, 1_000, 10_000])("a refused Delete (a calculator first, %i blocks after it) reads one target block", (n) => {
    load("```calc\n1+1\n```", n);
    const before = JSON.stringify(doc.byId.c1);
    const { handled, commits, outline } = pressDelete();
    expect(handled).toBe(true);
    expect(commits).toBe(0);
    expect(probe.firstInside).toBe(1);
    expect(outline).toEqual(["first"]);
    expect(JSON.stringify(doc.byId.c1)).toBe(before);
    expect(doc.byId.first).toBeDefined();
  });

  it.each([100, 1_000, 10_000])("a successful Delete (%i blocks after it) computes its target once", (n) => {
    load("Child", n);
    const { handled, commits, outline } = pressDelete();
    expect(handled).toBe(true);
    expect(commits).toBe(1);
    expect(probe.firstInside).toBe(1);
    expect(outline).toEqual(["first"]);
    expect(doc.byId.c1.raw).toBe("CellChild");
    expect(doc.byId.c1.children.length).toBe(n);
  });
});
