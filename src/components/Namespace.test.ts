import { describe, it, expect } from "vitest";
import { buildNamespaceTree, namespaceHierarchyRows } from "./Namespace";

describe("buildNamespaceTree", () => {
  it("nests pages by '/' segments, ignoring non-namespaced names", () => {
    const tree = buildNamespaceTree(["a/b/c", "a/b/d", "a/e", "plain", "z/y"]);
    expect(tree.map((n) => n.seg)).toEqual(["a", "z"]); // "plain" excluded; sorted
    const a = tree[0];
    expect(a.full).toBe("a");
    expect(a.children.map((n) => n.seg)).toEqual(["b", "e"]);
    const b = a.children[0];
    expect(b.full).toBe("a/b");
    expect(b.children.map((n) => n.full)).toEqual(["a/b/c", "a/b/d"]);
  });

  it("creates intermediate nodes even without their own page", () => {
    // only "x/y/z" exists; "x" and "x/y" still appear as tree nodes
    const tree = buildNamespaceTree(["x/y/z"]);
    expect(tree[0].seg).toBe("x");
    expect(tree[0].children[0].seg).toBe("y");
    expect(tree[0].children[0].children[0].full).toBe("x/y/z");
  });
});

describe("namespaceHierarchyRows", () => {
  const all = [
    "Formula1",
    "Formula1/2026",
    "Formula1/2026/08 Austrian Grand Prix",
    "Formula1/2026/09 Italian Grand Prix",
    "Formula1/2025/12 Abu Dhabi Grand Prix", // no "Formula1/2025" page of its own
    "Other",
  ];

  it("emits one row per descendant LEVEL, synthesizing missing intermediates", () => {
    const rows = namespaceHierarchyRows(all, "Formula1").map((r) => r.join("/"));
    expect(rows).toEqual([
      "Formula1/2025", // synthesized — no file of its own
      "Formula1/2025/12 Abu Dhabi Grand Prix",
      "Formula1/2026",
      "Formula1/2026/08 Austrian Grand Prix",
      "Formula1/2026/09 Italian Grand Prix",
    ]);
  });

  it("does not include the page itself, only descendants", () => {
    const rows = namespaceHierarchyRows(all, "Formula1/2026").map((r) => r.join("/"));
    expect(rows).toEqual([
      "Formula1/2026/08 Austrian Grand Prix",
      "Formula1/2026/09 Italian Grand Prix",
    ]);
  });

  it("a namespaced leaf with no descendants shows its parent path", () => {
    const rows = namespaceHierarchyRows(all, "Formula1/2026/08 Austrian Grand Prix");
    expect(rows).toEqual([["Formula1", "2026"]]);
  });

  it("a plain (non-namespaced) page with no descendants → nothing", () => {
    expect(namespaceHierarchyRows(all, "Other")).toEqual([]);
  });
});
