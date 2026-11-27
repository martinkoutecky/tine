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

import { NamespaceCrumb, NamespaceHierarchy, NamespaceMacro, buildNamespaceTree } from "./Namespace";
import { createSignal } from "solid-js";
import { resetReferenceSectionState } from "../referenceSectionState";
import { allPageNames, namespaceTarget, patchPageIndex, refreshPageIndex, resetPageIndex } from "../pageIndex";
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

// the store's `parent` is the tree's one answer.
// A row without one has no parent (no prefix is derived here), and a day left
// without a parent is a top node even with nothing below it.
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
    // store accepts neither ISO prefix (equal depth), so neither row has one.
    const names = await hold([journal("2026-02-11/0", 20261102), journal("2026-11-02/0", 20260211)]);
    expect(namespaceTarget("2026-02-11/0", true)).toBeNull();
    expect(shape(names)).toEqual([["2026-02-11/0", "journal", []], ["2026-11-02/0", "journal", []]]);
    expect(shape(await hold([journal("2026-11-02/0", 20261102)]))).toEqual([["2026-11-02/0", "journal", []]]);
  });

  it("nests a day under its accepted ordinary prefix and not the reverse (mixed cycle)", async () => {
    const names = await hold([
      journal("2026-11-02/0/0", 20261102, { parent: { name: "2026-11-02/0", kind: "page" } }),
      ordinary("2026-11-02/0", { namespace_parent: true }),
    ]);
    expect(shape(names)).toEqual([["2026-11-02/0", "page", ["0"]]]);
  });

  it("keeps a day whose month is only an alias as a top node, and lists no alias node", async () => {
    const names = await hold([
      journal("2026/11/02", 20261102, { parent: { name: "2026/11", kind: "page" } }),
      { ...ordinary("2026/11", { namespace_excluded: true }), target: { kind: "alias", owners: ["pages/Owner.md"] } },
      { ...ordinary("Owner"), target: { kind: "existing", id: "pages/Owner.md", others: [] } },
    ]);
    expect(shape(names)).toEqual([["2026/11/02", "journal", []]]);
  });

  it("drops an ordinary row without a parent and its crumb opens what the prefix links to", async () => {
    setJournalFormats("yyyy-dd-MM/0", "yyyy_MM_dd");
    const names = await hold([journal("2026-02-11/0", 20261102), ordinary("2026-11-02/notes")]);
    expect(shape(names)).toEqual([["2026-02-11/0", "journal", []]]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <NamespaceCrumb name="2026-11-02/notes" />, root);
    try {
      root.querySelector<HTMLElement>(".ns-crumb-item")!.click();
      expect(route()).toMatchObject({ kind: "page", name: "2026-02-11/0", pageKind: "journal" });
    } finally { dispose(); }
  });

  it("attaches a blank day once a delta lists its month, and detaches it when the month goes", async () => {
    const day = journal("2026/12/01", 20261201, { parent: { name: "2026/12", kind: "page" } });
    await hold([day], "20");
    const tree = () => shape(allPageNames());
    expect(tree()).toEqual([["2026/12/01", "journal", []]]);
    expect(patchPageIndex(21n, { keys: ["2026/12"], entries: [ordinary("2026/12")] })).not.toBeNull();
    expect(tree()).toEqual([["2026/12", "page", ["01"]]]);
    expect(patchPageIndex(22n, { keys: ["2026/12"], entries: [] })).not.toBeNull();
    expect(tree()).toEqual([["2026/12/01", "journal", []]]);
  });
});
