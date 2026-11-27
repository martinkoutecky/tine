import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { rightSidebar, setRightSidebar } from "../ui";
import { installPageIndex, linkTarget, refreshPageIndex, resetPageIndex } from "../pageIndex";
import { PageView } from "./Page";
import { setGraphMeta } from "../graphSession";
import type { PageInventoryEntry, PageRead } from "../types";

// GH #1052: a nested page reference `[[Lorem [[Impsum]]]]` (what completion
// inserts for the page `Lorem [[Impsum]]`) opens the page each part names, as
// Logseq's `nested-link` does: the outer label `Lorem [[Impsum]]`, the inner
// link `Impsum`. It used to ask for a page literally named with the outer pair.

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  resetPageIndex();
  setGraphMeta(null);
  setRightSidebar([]);
  resetTabsToJournals();
  document.body.replaceChildren();
});

const page = (name: string, id: string, raw: string): PageRead => ({
  name, title: name, kind: "page", id, pre_block: null, rev: "r",
  blocks: [{ id: `${name}-root`, raw, children: [], collapsed: false }],
});

function serve(extra: PageRead[] = []) {
  const pages = [
    page("Source", "pages/Source.md", "See [[Lorem [[Impsum]]]] here"),
    page("Lorem [[Impsum]]", "pages/Lorem [[Impsum]].md", "Seeded outer body"),
    page("Impsum", "pages/Impsum.md", "Seeded inner body"),
    ...extra,
  ];
  return vi.spyOn(backend(), "getPage").mockImplementation(async (name) => pages.find((p) => p.name === name) ?? null);
}

function mountSource() {
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Source", pageKind: "page" });
  const root = document.createElement("div");
  document.body.append(root);
  return { root, dispose: render(() => <PageView />, root) };
}

const links = (root: HTMLElement) => [...root.querySelectorAll<HTMLAnchorElement>(".ls-block a.page-ref")];
const click = (el: Element, init: MouseEventInit = {}) =>
  el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ...init }));

it("a click on the outer label opens the page the outer part names", async () => {
  const read = serve();
  const { root, dispose } = mountSource();
  try {
    await vi.waitFor(() => expect(links(root).length).toBeGreaterThan(0));
    click(links(root)[0]);
    await vi.waitFor(() => expect(mainPaneRouter.route()).not.toMatchObject({ name: "Source" }));
    expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Lorem [[Impsum]]" });
    await vi.waitFor(() => expect(root.textContent).toContain("Seeded outer body"));
    expect(read.mock.calls.map(([name]) => name)).not.toContain("[[Lorem [[Impsum]]]]");
  } finally { dispose(); }
});

it("the inner reference is a part of the one link and a click there opens its own page", async () => {
  serve();
  const { root, dispose } = mountSource();
  try {
    await vi.waitFor(() => expect(links(root)).toHaveLength(1));
    const inner = links(root)[0].querySelector("[data-nested-range]")!;
    expect(inner.textContent).toBe("[[Impsum]]");
    click(inner);
    await vi.waitFor(() => expect(root.textContent).toContain("Seeded inner body"));
    expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Impsum" });
  } finally { dispose(); }
});

it("a Shift-click on the inner part opens the inner page in the sidebar", async () => {
  serve();
  const { root, dispose } = mountSource();
  try {
    await vi.waitFor(() => expect(links(root)).toHaveLength(1));
    click(links(root)[0].querySelector("[data-nested-range]")!, { shiftKey: true });
    expect(rightSidebar().map((item) => (item.kind === "page" ? item.name : item.kind))).toEqual(["Impsum"]);
    expect(mainPaneRouter.route()).toMatchObject({ name: "Source" });
  } finally { dispose(); }
});

it("a Shift-click on the outer label opens the outer page in the sidebar", async () => {
  serve();
  const { root, dispose } = mountSource();
  try {
    await vi.waitFor(() => expect(links(root)).toHaveLength(1));
    click(links(root)[0], { shiftKey: true });
    expect(rightSidebar().map((item) => (item.kind === "page" ? item.name : item.kind))).toEqual(["Lorem [[Impsum]]"]);
    expect(mainPaneRouter.route()).toMatchObject({ name: "Source" });
  } finally { dispose(); }
});

it("an outer name that is an alias opens its owner", async () => {
  const entry = (key: string, name: string, target: PageInventoryEntry["target"], link?: PageInventoryEntry["link"]): PageInventoryEntry =>
    ({ key, name, is_journal: false, day: null, target, ...(link ? { link } : {}) });
  resetPageIndex();
  vi.spyOn(backend(), "graphBindingGeneration").mockReturnValue(1);
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [
    entry("canonical", "Canonical", { kind: "existing", id: "pages/Canonical.md", others: [] }),
    entry("lorem [[impsum]]", "Lorem [[Impsum]]", { kind: "alias", owners: ["pages/Canonical.md"] }, { name: "Canonical", kind: "page" }),
    entry("impsum", "Impsum", { kind: "existing", id: "pages/Impsum.md", others: [] }),
  ] });
  installPageIndex();
  await refreshPageIndex();
  await vi.waitFor(() => expect(linkTarget("Lorem [[Impsum]]").name).toBe("Canonical"));
  const read = serve([page("Canonical", "pages/Canonical.md", "Canonical body")]);
  const { root, dispose } = mountSource();
  try {
    await vi.waitFor(() => expect(links(root)).toHaveLength(1));
    await vi.waitFor(() => expect(links(root)[0].hasAttribute("data-missing-page")).toBe(false));
    click(links(root)[0]);
    await vi.waitFor(() => expect(root.textContent).toContain("Canonical body"));
    expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Canonical" });
    expect(read.mock.calls.map(([name]) => name)).not.toContain("[[Lorem [[Impsum]]]]");
  } finally { dispose(); }
});
