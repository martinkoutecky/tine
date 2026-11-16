// D-20261019-AL1/AL2 (card hB9Zjw): the page header tells the names an alias
// group has (no file of their own) from the other real pages in the group,
// and each opens the right page.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PageView } from "./Page";
import { RightSidebar } from "./RightSidebar";
import { applySidebarSession } from "../ui";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { resetStore } from "../document";
import { mainPaneRouter, resetTabsToJournals, route } from "../router";
import { bumpPageInventoryRev } from "../graphSession";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import type { AliasGroup, PageInventoryEntry } from "../types";

vi.mock("./LinkedReferences", () => ({ LinkedReferences: () => <div class="linked-references">Linked References</div> }));
vi.mock("./UnlinkedReferences", () => ({ UnlinkedReferences: () => <div>Unlinked References</div> }));
vi.mock("./TaggedPages", () => ({ TaggedPages: () => null }));
beforeAll(initParser);
afterEach(() => {
  applySidebarSession({ right: false, items: [] });
  resetStore();
  resetTabsToJournals();
  resetPageIndex();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

const existing = (name: string): PageInventoryEntry => ({
  key: name.toLowerCase(), name, is_journal: false, day: null,
  target: { kind: "existing", id: `pages/${name}.md`, others: [] },
});
const alias = (name: string, owners: string[]): PageInventoryEntry => ({
  key: name.toLowerCase(), name, is_journal: false, day: null,
  target: { kind: "alias", owners: owners.map((owner) => `pages/${owner}.md`) },
});

/** A chain Alpha -alias-> Beta -alias-> Gamma, with fileless `al` (Alpha's)
 *  and `be` (Beta's), and an unrelated page. */
async function chainGraph() {
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({
    rev: "1",
    entries: [existing("Alpha"), existing("Beta"), existing("Gamma"), existing("Lonely"), alias("al", ["Alpha"]), alias("be", ["Beta"])],
  });
  await refreshPageIndex();
  const page = (name: string, pre_block: string | null) => ({ id: `pages/${name}.md`, name, title: name, kind: "page" as const, pre_block, blocks: [] });
  const pages = { Alpha: page("Alpha", "alias:: al, Beta"), Beta: page("Beta", "alias:: be, Gamma"), Gamma: page("Gamma", null), Lonely: page("Lonely", null) };
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => pages[name as keyof typeof pages] ?? null);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async (path) => Object.values(pages).find((dto) => dto.id === path) ?? null);
  for (const dto of Object.values(pages)) loadSingle(dto);
  const groups: Record<string, AliasGroup> = {
    Alpha: { aliases: ["al", "be"], pages: ["Beta", "Gamma"] },
    Beta: { aliases: ["al", "be"], pages: ["Alpha", "Gamma"] },
    Gamma: { aliases: ["al", "be"], pages: ["Alpha", "Beta"] },
  };
  return vi.spyOn(backend(), "pageAliasGroup").mockImplementation(async (name) => groups[name] ?? { aliases: [], pages: [] });
}

const names = (host: HTMLElement, selector: string) =>
  [...host.querySelectorAll(`${selector} a.page-ref`)].map((link) => link.textContent);

function mountMain(name: string) {
  mainPaneRouter.openPage(name, "page");
  const host = document.createElement("div");
  document.body.append(host);
  return { host, dispose: render(() => <PageView />, host) };
}

it("a page that declares nothing still shows its group: fileless names apart from the real pages", async () => {
  const group = await chainGraph();
  const { host, dispose } = mountMain("Gamma");
  try {
    await vi.waitFor(() => expect(names(host, "[data-alias-group-pages]")).toEqual(["Alpha", "Beta"]));
    expect(names(host, "[data-alias-names]")).toEqual(["al", "be"]);
    expect(host.querySelector("[data-alias-names]")?.textContent).toContain("Also called");
    expect(host.querySelector("[data-alias-group-pages]")?.textContent).toContain("Related pages in this alias group");
    expect(group).toHaveBeenCalledWith("Gamma");
  } finally { dispose(); }
});

it("a real page in the group opens its own file; a fileless name opens the page declaring it", async () => {
  await chainGraph();
  const { host, dispose } = mountMain("Gamma");
  try {
    const link = (selector: string, text: string) =>
      [...host.querySelectorAll<HTMLElement>(`${selector} a.page-ref`)].find((el) => el.textContent === text)!;
    await vi.waitFor(() => expect(link("[data-alias-group-pages]", "Beta")).toBeTruthy());
    link("[data-alias-group-pages]", "Beta").click();
    expect(route()).toMatchObject({ kind: "page", name: "Beta" });
    mainPaneRouter.openPage("Gamma", "page");
    await vi.waitFor(() => expect(link("[data-alias-names]", "al")).toBeTruthy());
    link("[data-alias-names]", "al").click();
    expect(route()).toMatchObject({ kind: "page", name: "Alpha" });
  } finally { dispose(); }
});

it("the page's own declared names show at once, sorted into names and real pages by the page index", async () => {
  await chainGraph();
  vi.spyOn(backend(), "pageAliasGroup").mockImplementation(() => new Promise(() => {}));
  const { host, dispose } = mountMain("Alpha");
  try {
    await vi.waitFor(() => expect(names(host, "[data-alias-names]")).toEqual(["al"]));
    expect(names(host, "[data-alias-group-pages]")).toEqual(["Beta"]);
  } finally { dispose(); }
});

it("a page outside any alias group gets no alias rows", async () => {
  const group = await chainGraph();
  const { host, dispose } = mountMain("Lonely");
  try {
    await vi.waitFor(() => expect(group).toHaveBeenCalledWith("Lonely"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(host.querySelector("h1.page-title")?.textContent).toBe("Lonely");
    expect(host.querySelector(".page-aliases")).toBeNull();
  } finally { dispose(); }
});

it("the right sidebar shows the same rows, and an alias edit elsewhere refreshes them", async () => {
  const group = await chainGraph();
  applySidebarSession({ right: true, items: [{ kind: "page", name: "Gamma", pageKind: "page" }] });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <RightSidebar />, host);
  try {
    await vi.waitFor(() => expect(names(host, "[data-alias-group-pages]")).toEqual(["Alpha", "Beta"]));
    group.mockResolvedValue({ aliases: ["al"], pages: ["Beta"] });
    bumpPageInventoryRev();
    await vi.waitFor(() => expect(names(host, "[data-alias-group-pages]")).toEqual(["Beta"]));
    expect(names(host, "[data-alias-names]")).toEqual(["al"]);
  } finally { dispose(); }
});
