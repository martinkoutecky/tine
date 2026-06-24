import { describe, it, expect } from "vitest";
import { buildNamespaceTree } from "./Namespace";

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
