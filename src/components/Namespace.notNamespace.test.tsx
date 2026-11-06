// UI-CM-NS-VIEWS-RELATIVE-URL-NAMES (CM step 2 batch 15, model round 10 S15):
// `./x`, `../x/y` and a URL with an origin are not namespace names in Logseq
// (`namespace-page?`), so the namespace views must not nest them under `.`,
// `../x` or `https://a.b`. The store says which names those are
// (`not_namespace` on the inventory row, `refs::is_namespace_name`); the views
// read it and test no spelling themselves.
import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

const backendMock = vi.hoisted(() => {
  const row = (name: string, existing: boolean, notNamespace = false) => ({
    key: name.toLowerCase(),
    name,
    is_journal: false,
    day: null,
    target: existing
      ? { kind: "existing" as const, id: `pages/${encodeURIComponent(name)}.md`, others: [] }
      : { kind: "absent" as const, id: `pages/${encodeURIComponent(name)}.md` },
    ...(notNamespace ? { not_namespace: true } : {}),
  });
  return {
    graphBindingGeneration: () => 0,
    pageInventory: vi.fn(async () => ({
      rev: "1",
      entries: [
        row(".", true),
        row("./x", true, true),
        row("../x", false, true),
        row("../x/y", false, true),
        row("a", false),
        row("a/b", true),
        row("old", false),
        { ...row("old/sub", false), target: { kind: "alias", owners: ["pages/owner.md"] }, namespace_excluded: true },
        row("real", false),
        row("real/alias", true),
        row("refs", false),
        row("refs/only", false),
        row("https://a.b", false, true),
        row("https://a.b/c", false, true),
      ],
    })),
  };
});

vi.mock("../backend", () => ({ backend: () => backendMock }));

import { NamespaceCrumb, NamespaceHierarchy, NamespaceTree } from "./Namespace";
import { resetReferenceSectionState } from "../referenceSectionState";

afterEach(() => {
  resetReferenceSectionState();
  document.body.innerHTML = "";
});

const mount = (view: () => import("solid-js").JSX.Element) => {
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(view, root) };
};

// Loads the shared inventory: `a/b`'s own Hierarchy row appears once it is held.
const loaded = async () => {
  const { root, dispose } = mount(() => <NamespaceHierarchy name="a/b" />);
  await vi.waitFor(() => expect(root.querySelectorAll(".ns-hier-row")).toHaveLength(1));
  dispose();
};

describe("namespace views ask the store which names are namespaces", () => {
  it("the tree nests only namespace names", async () => {
    await loaded();
    const { root, dispose } = mount(() => <NamespaceTree />);
    try {
      await vi.waitFor(() => expect([...root.querySelectorAll(".ns-node-label")].map((n) => n.textContent)).toEqual(["a", "b", "real", "alias", "refs", "only"]));
    } finally { dispose(); }
  });

  it("alias-only names have no hierarchy, while file and reference pages remain", async () => {
    await loaded();
    const ns = await import("./Namespace");
    const index = await import("../pageIndex");
    expect(index.allPageNames()).toContain("old/sub");
    expect(ns.namespaceHierarchyRows(index.allPageNames(), "old")).toEqual([]);
    expect(ns.namespaceHierarchyRows(index.allPageNames(), "old/sub")).toEqual([]);
    expect(ns.namespaceNode(index.allPageNames(), "old/sub")).toBeNull();
    expect(ns.namespaceHierarchyRows(index.allPageNames(), "real")).toEqual([["real", "alias"]]);
    expect(ns.namespaceHierarchyRows(index.allPageNames(), "refs")).toEqual([["refs", "only"]]);
  });

  it("the Hierarchy section of `.`, `./x`, `../x/y` and a URL is empty; `a` keeps its own", async () => {
    await loaded();
    for (const [name, want] of [[".", 0], ["./x", 0], ["../x", 0], ["../x/y", 0], ["https://a.b", 0], ["https://a.b/c", 0], ["a", 1]] as const) {
      const { root, dispose } = mount(() => <NamespaceHierarchy name={name} />);
      try {
        expect(root.querySelectorAll(".ns-hier-row"), name).toHaveLength(want);
      } finally { dispose(); }
    }
  });

  it("no namespace crumb above `./x` or a URL", async () => {
    await loaded();
    for (const [name, crumb] of [["./x", false], ["../x/y", false], ["https://a.b/c", false], ["a/b", true]] as const) {
      const { root, dispose } = mount(() => <NamespaceCrumb name={name} />);
      try {
        expect(root.querySelector(".ns-crumb") !== null, name).toBe(crumb);
      } finally { dispose(); }
    }
  });
});
