import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { PageInventory } from "../types";

const backendMock = vi.hoisted(() => {
  const referenced = (name: string) => ({
    key: name.toLowerCase(),
    name,
    is_journal: false,
    day: null,
    target: { kind: "absent" as const, id: `pages/${name.replaceAll("/", "___")}.md` },
    // The store's tree parent: the immediate prefix page.
    parent: { name: name.slice(0, name.lastIndexOf("/")), kind: "page" as const },
  });
  return {
    graphBindingGeneration: () => 0,
    pageInventory: vi.fn(async (): Promise<PageInventory> => ({
      rev: "1",
      entries: [
        {
          key: "test",
          name: "test",
          is_journal: false,
          day: null,
          target: { kind: "existing" as const, id: "pages/test.md", others: [] },
        },
        referenced("test/testy test"),
        referenced("test/testy test/another"),
        referenced("test/testy tester"),
      ],
    })),
  };
});

vi.mock("../backend", () => ({ backend: () => backendMock }));

import { NamespaceCrumb, NamespaceHierarchy, NamespaceMacro, buildNamespaceTree, type NsNode } from "./Namespace";
import { createSignal } from "solid-js";
import { resetReferenceSectionState } from "../referenceSectionState";
import { allPageNames, namespaceRow, patchPageIndex, refreshPageIndex, resetPageIndex } from "../pageIndex";
import { route } from "../router";
import { setJournalFormats } from "../journal";
import { setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";

afterEach(() => {
  setGraphMeta(null);
  resetReferenceSectionState();
  document.body.innerHTML = "";
});

// L7s6-04: OG's header is the namespace the macro names, and the tree is its descendants.
describe("{{namespace a/b}}", () => {
  it("heads with the named namespace and lists only what lies below it", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <NamespaceMacro root="test/testy test" />, root);
    try {
      await vi.waitFor(() => expect(root.querySelector(".ns-macro-head .page-ref")?.textContent).toBe("test/testy test"));
      expect([...root.querySelectorAll(".ns-macro-tree .page-ref")].map((a) => a.textContent)).toEqual(["another"]);
    } finally { dispose(); }
  });
});

describe("GH #229 reference-only namespace descendants", () => {
  it("GH #959: follows Show brackets in every Hierarchy segment, including a live setting change", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    setGraphMeta({ show_brackets: false } as GraphMeta);
    const dispose = render(() => <NamespaceHierarchy name="test" />, root);
    try {
      await vi.waitFor(() => expect(root.querySelectorAll(".ns-hier-row")).toHaveLength(3));
      const labels = () => [...root.querySelectorAll(".ns-hier-row .page-ref")].map(link => link.textContent);
      const inventoryReads = backendMock.pageInventory.mock.calls.length;
      expect(labels()).toEqual(["test", "testy test", "test", "testy test", "another", "test", "testy tester"]);
      setGraphMeta({ show_brackets: true } as GraphMeta);
      expect(labels()).toEqual(["[[test]]", "[[testy test]]", "[[test]]", "[[testy test]]", "[[another]]", "[[test]]", "[[testy tester]]"]);
      setGraphMeta({ show_brackets: false } as GraphMeta);
      expect(root.querySelector(".bracket")).toBeNull();
      expect(backendMock.pageInventory).toHaveBeenCalledTimes(inventoryReads);
    } finally { dispose(); }
  });
  it("GH #738: folds by title, retains the page choice across navigation/remount and resets with the graph", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const [name, setName] = createSignal("test");
    let dispose = render(() => <NamespaceHierarchy name={name()} />, root);
    const title = () => root.querySelector<HTMLButtonElement>('button[aria-expanded]')!;
    try {
      await vi.waitFor(() => expect(root.querySelectorAll(".ns-hier-row")).toHaveLength(3));
      expect(title()?.textContent).toContain("Hierarchy");
      expect(title()?.getAttribute("aria-expanded")).toBe("true");
      title().focus();
      expect(document.activeElement).toBe(title());
      // Native button activation supports Enter/Space without a custom key handler.
      title().click();
      expect(title().getAttribute("aria-expanded")).toBe("false");
      expect(root.querySelector(".ns-hier-row")).toBeNull();
      setName("test/testy test");
      expect(title().getAttribute("aria-expanded")).toBe("true");
      setName("test");
      expect(title().getAttribute("aria-expanded")).toBe("false");
      dispose();
      dispose = render(() => <NamespaceHierarchy name="test" />, root);
      expect(title().getAttribute("aria-expanded")).toBe("false");
      title().click();
      expect(root.querySelectorAll(".ns-hier-row")).toHaveLength(3);
      title().click();
      dispose();
      resetReferenceSectionState();
      dispose = render(() => <NamespaceHierarchy name="test" />, root);
      expect(title().getAttribute("aria-expanded")).toBe("true");
    } finally { dispose(); }
  });
  it("shows the synthesized reference-only descendants in the real Hierarchy component", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <NamespaceHierarchy name="test" />, root);

    try {
      await vi.waitFor(() => {
        const rows = [...root.querySelectorAll(".ns-hier-row")].map((row) =>
          [...row.querySelectorAll(".page-ref")]
            .map((link) => link.textContent?.replaceAll("[[", "").replaceAll("]]", ""))
            .join("/")
        );
        expect(rows).toEqual([
          "test/testy test",
          "test/testy test/another",
          "test/testy tester",
        ]);
      });
    } finally {
      dispose();
    }
  });
});

it("attaches a date-prefixed child to the typed journal parent in the macro and hierarchy", async () => {
  const canonical = "2026_23_10";
  backendMock.pageInventory.mockResolvedValueOnce({ rev: "2", entries: [
    { key: canonical, name: canonical, is_journal: true, day: 20261023,
      target: { kind: "absent", id: "journals/23.10.2026.md" }, namespace_parent: true },
    { key: "23.10.2026/child", name: "23.10.2026/child", is_journal: false, day: null,
      target: { kind: "absent", id: "pages/23.10.2026___child.md" }, parent: { name: canonical, kind: "journal" } },
  ] });
  resetPageIndex();
  await refreshPageIndex();
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <><NamespaceMacro root={canonical} /><NamespaceHierarchy name={canonical} /></>, root);
  try {
    await vi.waitFor(() => expect(root.querySelector(".ns-macro-tree .page-ref")?.textContent).toBe("child"));
    expect(root.querySelectorAll(".ns-hier-row")).toHaveLength(1);
    expect(root.querySelector(".ns-hier-row")?.textContent).toContain("child");
    root.querySelector<HTMLAnchorElement>(".ns-macro-head .page-ref")!.click();
    expect(route()).toMatchObject({ kind: "page", name: canonical, pageKind: "journal" });
  } finally {
    dispose();
    resetPageIndex();
  }
});

it("keeps same-title ordinary and virtual journal namespace parents distinct when navigating", async () => {
  const canonical = "2026_10_23";
  setJournalFormats("yyyy_MM_dd", "");
  const entries: PageInventory["entries"] = [
    { key: canonical, name: canonical, is_journal: false, day: null,
      target: { kind: "existing", id: `pages/${canonical}.md`, others: [] } },
    { key: canonical, name: canonical, is_journal: true, day: 20261023,
      target: { kind: "absent", id: "journals/2026_10_23.md" }, namespace_parent: true,
      link: { name: "2026-10-23", kind: "journal" } },
    { key: "2026-10-23/child", name: "2026-10-23/child", is_journal: false, day: null,
      target: { kind: "absent", id: "pages/2026-10-23___child.md" }, parent: { name: canonical, kind: "journal" } },
    { key: `${canonical}/literal`, name: `${canonical}/literal`, is_journal: false, day: null,
      target: { kind: "absent", id: `pages/${canonical}___literal.md` }, parent: { name: canonical, kind: "page" } },
  ];
  backendMock.pageInventory.mockResolvedValueOnce({ rev: "3", entries });
  resetPageIndex();
  await refreshPageIndex();
  const tree = buildNamespaceTree(entries.map(entry => entry.name));
  expect(tree.map(node => [node.full, node.kind ?? "page", node.children.map(child => child.seg)]))
    .toEqual([[canonical, "journal", ["child"]], [canonical, "page", ["literal"]]]);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <><NamespaceMacro root="2026-10-23" /><NamespaceHierarchy name="2026-10-23/child" /></>, root);
  try {
    await vi.waitFor(() => expect(root.querySelector(".ns-macro-tree .page-ref")?.textContent).toBe("child"));
    root.querySelector<HTMLAnchorElement>(".ns-macro-head .page-ref")!.click();
    expect(route()).toMatchObject({ kind: "page", name: "2026-10-23", pageKind: "journal" });
    root.querySelector<HTMLAnchorElement>(".ns-hier-row .page-ref")!.click();
    expect(route()).toMatchObject({ kind: "page", name: "2026-10-23", pageKind: "journal" });
  } finally {
    dispose();
    resetPageIndex();
    setJournalFormats("", "");
  }
});

// The store's `parent` is each row's candidate; the tree attaches the row
// below it exactly when that typed row is listed. A row without one has no
// parent (no prefix is derived here), and a `namespace_day` left without a
// parent is a top node even with nothing below it.
describe("slash days in the namespace tree", () => {
  type Entry = PageInventory["entries"][number];
  const journal = (name: string, day: number, extra: Partial<Entry> = {}): Entry => ({
    key: name.toLowerCase(), name, is_journal: true, day,
    target: { kind: "existing", id: `journals/${day}.md`, others: [] }, ...extra });
  const ordinary = (name: string, extra: Partial<Entry> = {}): Entry => ({
    key: name.toLowerCase(), name, is_journal: false, day: null,
    target: { kind: "absent", id: `pages/${name.replaceAll("/", "___")}.md` }, ...extra });
  async function hold(entries: Entry[], rev = "10") {
    backendMock.pageInventory.mockResolvedValueOnce({ rev, entries });
    resetPageIndex();
    await refreshPageIndex();
    return entries.map(entry => entry.name);
  }
  const shape = (names: string[]) => buildNamespaceTree(names)
    .map(node => [node.full, node.kind ?? "page", node.children.map(child => child.seg)]);
  afterEach(() => { resetPageIndex(); setJournalFormats("", ""); });

  it("keeps both swapped-slash days and a same-day-prefix day as childless top nodes", async () => {
    // yyyy-dd-MM/0: 2 Nov shows 2026-02-11/0 and 11 Feb 2026-11-02/0; the
    // store accepts neither ISO prefix (equal depth): no candidate.
    const root = { namespace_day: true };
    const names = await hold([journal("2026-02-11/0", 20261102, root), journal("2026-11-02/0", 20260211, root)]);
    expect(namespaceRow("2026-02-11/0", "journal")?.parent).toBeUndefined();
    expect(shape(names)).toEqual([["2026-02-11/0", "journal", []], ["2026-11-02/0", "journal", []]]);
    expect(shape(await hold([journal("2026-11-02/0", 20261102, root)]))).toEqual([["2026-11-02/0", "journal", []]]);
  });

  it("shows a childless day only when the store marks it a namespace day, whatever its spelling", async () => {
    // `//yyyy-MM-dd`, `///yyyy-MM-dd`: every prefix folds to an empty key, so
    // the store marks no namespace day; the tree tests no spelling of its own.
    expect(shape(await hold([journal("//2026-11-02", 20261102, { key: "/2026-11-02" }), journal("///2026-11-03", 20261103, { key: "//2026-11-03" })]))).toEqual([]);
    expect(shape(await hold([journal("2026-11-02/0", 20261102)]))).toEqual([]);
  });

  it("nests a day under its accepted ordinary prefix and not the reverse (mixed cycle)", async () => {
    const names = await hold([
      journal("2026-11-02/0/0", 20261102, { parent: { name: "2026-11-02/0", kind: "page" } }),
      ordinary("2026-11-02/0", { namespace_parent: true }),
    ]);
    expect(shape(names)).toEqual([["2026-11-02/0", "page", ["0"]]]);
  });

  it("keeps a day whose month candidate is only an alias as a top node, and lists no alias node", async () => {
    const names = await hold([
      journal("2026/11/02", 20261102, { namespace_day: true, parent: { name: "2026/11", kind: "page" } }),
      { ...ordinary("2026/11", { namespace_excluded: true }), target: { kind: "alias", owners: ["pages/Owner.md"] } },
      { ...ordinary("Owner"), target: { kind: "existing", id: "pages/Owner.md", others: [] } },
    ]);
    expect(shape(names)).toEqual([["2026/11/02", "journal", []]]);
  });

  it("drops an ordinary row without a parent and its crumb opens what the prefix links to", async () => {
    setJournalFormats("yyyy-dd-MM/0", "yyyy_MM_dd");
    const names = await hold([journal("2026-02-11/0", 20261102, { namespace_day: true }), ordinary("2026-11-02/notes")]);
    expect(shape(names)).toEqual([["2026-02-11/0", "journal", []]]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <NamespaceCrumb name="2026-11-02/notes" />, root);
    try {
      root.querySelector<HTMLElement>(".ns-crumb-item")!.click();
      expect(route()).toMatchObject({ kind: "page", name: "2026-02-11/0", pageKind: "journal" });
    } finally { dispose(); }
  });

  // The month a reference adds and removes travels as its own rows in a delta;
  // the day's row, which no delta carries, keeps its candidate.
  const month = (name: string): Entry[] => [
    ordinary(name, { parent: { name: "2026", kind: "page" } }),
    ordinary("2026", { namespace_parent: true }),
  ];

  it("moves a blank spaced-title day under its folded month by the patch, and back to the top", async () => {
    // `yyyy/MM /dd`: the day's prefix `2026/11 ` folds to the month `2026/11`.
    const tree = () => shape(allPageNames());
    await hold([journal("2026/11 /02", 20261102, { namespace_day: true, parent: { name: "2026/11 ", kind: "page" } })], "20");
    const reads = backendMock.pageInventory.mock.calls.length;
    expect(tree()).toEqual([["2026/11 /02", "journal", []]]);
    expect(patchPageIndex(21n, { keys: ["2026/11", "2026"], entries: month("2026/11") })).not.toBeNull();
    expect(tree()).toEqual([["2026", "page", ["11"]]]);
    expect(buildNamespaceTree(allPageNames())[0].children[0].children.map(child => [child.full, child.kind, child.seg]))
      .toEqual([["2026/11 /02", "journal", "02"]]);
    expect(patchPageIndex(22n, { keys: ["2026/11", "2026"], entries: [] })).not.toBeNull();
    expect(tree()).toEqual([["2026/11 /02", "journal", []]]);
    expect(backendMock.pageInventory.mock.calls.length).toBe(reads);
  });

  it("nests blank pages under a date-normalised day while the day is listed, and hides them otherwise", async () => {
    // `2026/11/2/Notes` and `2026/1/02/Notes` name the days `2026/11/02` and `2026/01/02`.
    const tree = () => buildNamespaceTree(allPageNames());
    const labels = (nodes: NsNode[]): unknown[] => nodes.map(node => [node.seg, labels(node.children)]);
    const notes = (name: string, day: string) => ({ ...ordinary(name, { parent: { name: day, kind: "journal" } }),
      target: { kind: "existing" as const, id: `pages/${name.replaceAll("/", "___")}.md`, others: [] } });
    await hold([notes("2026/11/2/Notes", "2026/11/02"), notes("2026/1/02/Notes", "2026/01/02")], "30");
    const reads = backendMock.pageInventory.mock.calls.length;
    expect(tree()).toEqual([]);
    const days = (["2026/11", "2026/01"] as const).map(m => ({
      ...journal(`${m}/02`, Number(`${m.replace("/", "")}02`), { namespace_day: true, parent: { name: m, kind: "page" } }),
      target: { kind: "absent" as const, id: `journals/${m.replace("/", "_")}_02.md` } }));
    const delta = { keys: ["2026/11/02", "2026/01/02", "2026/11", "2026/01", "2026"],
      entries: [...days, ...month("2026/11"), ordinary("2026/01", { parent: { name: "2026", kind: "page" } })] };
    expect(patchPageIndex(31n, delta)).not.toBeNull();
    expect(labels(tree())).toEqual([["2026", [["01", [["02", [["Notes", []]]]]], ["11", [["02", [["Notes", []]]]]]]]]);
    expect(patchPageIndex(32n, { keys: delta.keys, entries: [] })).not.toBeNull();
    expect(tree()).toEqual([]);
    expect(backendMock.pageInventory.mock.calls.length).toBe(reads);
  });

  it("nests a name with a trailing slash under the prefix before it", async () => {
    // `a/b/` folds to `a/b`, so its candidate is `a`.
    const names = await hold([
      ordinary("a/b/", { key: "a/b", parent: { name: "a", kind: "page" } }),
      ordinary("a/c", { parent: { name: "a", kind: "page" } }),
      ordinary("a", { namespace_parent: true }),
    ]);
    expect(shape(names)).toEqual([["a", "page", ["b/", "c"]]]);
  });
});
