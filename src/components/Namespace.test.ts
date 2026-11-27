import { afterEach, describe, it, expect, vi } from "vitest";
import type { PageInventoryEntry } from "../types";
const mode = vi.hoisted(() => ({ published: false }));
const inventory = vi.hoisted(() => ({ entries: [] as PageInventoryEntry[] }));
vi.mock("../publishedBackend", () => ({ isPublishedExport: () => mode.published }));
vi.mock("../backend", () => ({ backend: () => ({
  graphBindingGeneration: () => 0,
  pageInventory: async () => ({ rev: "1", entries: inventory.entries }),
}) }));
import { buildNamespaceTree, namespaceHierarchyRows, namespaceNode, namespaceRows } from "./Namespace";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import { ordinaryRows } from "../namespaceInventory.test.support";
afterEach(() => { mode.published = false; });

// The inventory lists every namespace parent (CM step 2 batch 10, model rule 1);
// these views nest what it lists and never add a level of their own. Each list
// is installed as the inventory first: the store answers every row's parent.
async function listed(names: string[]): Promise<string[]> {
  inventory.entries = ordinaryRows(names);
  resetPageIndex();
  await refreshPageIndex();
  return names;
}
const withParents = (names: string[]) => {
  const all = new Set(names);
  for (const name of names) {
    const segs = name.split("/");
    for (let k = 1; k < segs.length; k++) all.add(segs.slice(0, k).join("/"));
  }
  return [...all];
};

describe("buildNamespaceTree", () => {
  it("keeps frozen published v1's isolated slash leaves and hierarchy", () => {
    mode.published = true;
    expect(buildNamespaceTree(["./x"]).map((node) => node.full)).toEqual(["./x"]);
    expect(namespaceHierarchyRows(["./x"], "./x")).toEqual([["."]]);
    expect(buildNamespaceTree(["x", "x/y/z"])[0].children[0].full).toBe("x/y/z");
  });
  it("nests pages by '/' segments, ignoring non-namespaced names", async () => {
    const tree = buildNamespaceTree(await listed(withParents(["a/b/c", "a/b/d", "a/e", "plain", "z/y"])));
    expect(tree.map((n) => n.seg)).toEqual(["a", "z"]); // "plain" excluded; sorted
    const a = tree[0];
    expect(a.full).toBe("a");
    expect(a.children.map((n) => n.seg)).toEqual(["b", "e"]);
    const b = a.children[0];
    expect(b.full).toBe("a/b");
    expect(b.children.map((n) => n.full)).toEqual(["a/b/c", "a/b/d"]);
  });

  it("shows the namespace parents the inventory lists", async () => {
    const tree = buildNamespaceTree(await listed(["x/y/z", "x", "x/y"]));
    expect(tree[0].seg).toBe("x");
    expect(tree[0].children[0].seg).toBe("y");
    expect(tree[0].children[0].children[0].full).toBe("x/y/z");
  });

  it("does not bridge an absent immediate parent or list an isolated node (r11 C12)", async () => {
    expect(buildNamespaceTree(await listed(["x/y/z"]))).toEqual([]);
    expect(buildNamespaceTree(await listed(["x/y/z", "x"]))).toEqual([]);
    const tree = buildNamespaceTree(await listed(["x/y/z", "x/y/z/child", "x"]));
    expect(tree.map((n) => n.full)).toEqual(["x/y/z"]);
    expect(tree[0].children.map((n) => n.full)).toEqual(["x/y/z/child"]);
  });

  it("keeps the listed spelling of a parent, not a child's", async () => {
    const tree = buildNamespaceTree(await listed(["Team/Child", "team"]));
    expect(tree.map((n) => n.full)).toEqual(["team"]);
    expect(tree[0].children.map((n) => n.seg)).toEqual(["Child"]);
  });

  it("finds the {{namespace}} root only when the list holds it and something lies below", async () => {
    expect(namespaceNode(await listed(withParents(["a/b/c"])), "A/B")?.full).toBe("a/b");
    expect(namespaceNode(await listed(["a/b/c"]), "a")).toBeNull();
    expect(namespaceNode(await listed(withParents(["a/b/c"])), "a/b/c")).toBeNull();
  });
});

describe("namespaceHierarchyRows", () => {
  const all = [
    "Formula1",
    "Formula1/2026",
    "Formula1/2026/08 Austrian Grand Prix",
    "Formula1/2026/09 Italian Grand Prix",
    "Formula1/2025/12 Abu Dhabi Grand Prix",
    "Formula1/2025", // no file of its own: the inventory lists it as a namespace parent
    "Other",
  ];

  it("emits one row per listed name below the page, namespace parents included", async () => {
    const rows = namespaceHierarchyRows(await listed(all), "Formula1").map((r) => r.join("/"));
    expect(rows).toEqual([
      "Formula1/2025", // a namespace parent
      "Formula1/2025/12 Abu Dhabi Grand Prix",
      "Formula1/2026",
      "Formula1/2026/08 Austrian Grand Prix",
      "Formula1/2026/09 Italian Grand Prix",
    ]);
  });

  it("synthesizes no row the list does not hold", async () => {
    const rows = namespaceHierarchyRows(await listed(all.filter((n) => n !== "Formula1/2025")), "Formula1").map((r) => r.join("/"));
    expect(rows).not.toContain("Formula1/2025");
  });

  it("does not include the page itself, only descendants", async () => {
    const rows = namespaceHierarchyRows(await listed(all), "Formula1/2026").map((r) => r.join("/"));
    expect(rows).toEqual([
      "Formula1/2026/08 Austrian Grand Prix",
      "Formula1/2026/09 Italian Grand Prix",
    ]);
  });

  it("a namespaced leaf with no descendants shows its parent path", async () => {
    const rows = namespaceHierarchyRows(await listed(all), "Formula1/2026/08 Austrian Grand Prix");
    expect(rows).toEqual([["Formula1", "2026"]]);
  });

  it("a plain (non-namespaced) page with no descendants → nothing", async () => {
    expect(namespaceHierarchyRows(await listed(all), "Other")).toEqual([]);
  });
});


describe("namespace identity and depth (OG-B-FRONT)", () => {
  it("merges canonically equivalent ancestors and finds their descendants", async () => {
    const tree = buildNamespaceTree(await listed(["Café/one", "Cafe\u0301/two", "Café"]));
    expect(tree).toHaveLength(1);
    expect(tree[0].children.map(n => n.seg)).toEqual(["one", "two"]);
    expect(namespaceHierarchyRows(await listed(["Café", "Cafe\u0301/two"]), "Café")).toEqual([["Cafe\u0301", "two"]]);
  });
  it("sorts a deep imported namespace without recursion and keeps every level", async () => {
    const tree = buildNamespaceTree(await listed(withParents([Array(6000).fill("a").join("/")])));
    let level = tree, count = 0;
    while (level.length) { count++; level = level[0].children; }
    expect(count).toBe(6000);
    const rows = namespaceRows(tree);
    expect(rows).toHaveLength(6000);
    expect(rows.at(-1)?.depth).toBe(5999);
    expect(buildNamespaceTree(await listed(["root", ...Array.from({length: 2000}, (_, i) => `root/${i}`)]))[0].children).toHaveLength(2000);
  });
});
