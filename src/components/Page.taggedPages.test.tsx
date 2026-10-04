import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { TaggedPages } from "./TaggedPages";
import { RightSidebar } from "./RightSidebar";
import { applySidebarSession } from "../ui";
import { PageView } from "./Page";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { loadSingle } from "../document/workingSet";
import { resetStore } from "../document";
import { mainPaneRouter, resetTabsToJournals, route } from "../router";
import { bumpDataRev } from "../graphSession";
import type { QueryResult } from "../editor/queryIr";
vi.mock("./LinkedReferences", () => ({ LinkedReferences: () => <div class="linked-references">Linked References</div> }));
vi.mock("./UnlinkedReferences", () => ({ UnlinkedReferences: () => <div>Unlinked References</div> }));
beforeAll(initParser);
afterEach(() => { applySidebarSession({ right: false, items: [] }); resetStore(); resetTabsToJournals(); vi.restoreAllMocks(); document.body.replaceChildren(); });
const result = (names: string[]): QueryResult => ({ anchor: "page", pages: names.map(name => ({ name, kind: "page", path: `pages/${name}.md`, properties: [] })), total: names.length, exceeded: false, diagnostics: [], report: { supported: true, ran: [], ignored: [] } });
it("routed tag page lists tagged pages before references, navigates, and refreshes membership", async () => {
  const dto = { id: "pages/Research.md", name: "Research", title: "Research", kind: "page" as const, pre_block: null, blocks: [] };
  vi.spyOn(backend(), "getPage").mockImplementation(async name => ({ ...dto, name, title: name, id: `pages/${name}.md` }));
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => ({ ...dto, name: path.includes("Updated") ? "Updated" : "Research", id: path }));
  loadSingle(dto);
  vi.spyOn(backend(), "parseQuery").mockImplementation(async (text) => ({ query: { anchor: text.startsWith("(page-tags") ? "page" : "block", filter: { kind: "and", items: [] }, source: { kind: "builder" } }, view: {} }));
  let names = ["Zulu", "Alpha"];
  const run = vi.spyOn(backend(), "queryRun").mockImplementation(async query => query.anchor === "page" ? result(names) : { anchor: "block", groups: [], total: 0, exceeded: false, report: { supported: true, ran: [], ignored: [] } });
  mainPaneRouter.openPage(dto.name, "page");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".tagged-pages")?.textContent).toContain('Pages tagged with "Research"'));
    expect([...host.querySelectorAll(".tagged-pages li")].map(row => row.textContent)).toEqual(["Alpha", "Zulu"]);
    const section = host.querySelector(".tagged-pages")!;
    expect(section.compareDocumentPosition(host.querySelector(".linked-references:not(.tagged-pages)")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    names = ["Updated"]; bumpDataRev();
    await vi.waitFor(() => expect(section.textContent).toContain("Updated"));
    expect(section.textContent).not.toContain("Alpha");
    (section.querySelector("a.page-ref") as HTMLElement).click();
    expect(route()).toMatchObject({ kind: "page", name: "Updated" });
    expect(run).toHaveBeenCalled();
  } finally { dispose(); }
});

it("sidebar page uses the same tagged-page section, folds, and omits an empty result", async () => {
  const dto = { id: "pages/Research.md", name: "Research", kind: "page" as const, title: "Research", pre_block: null, blocks: [] };
  loadSingle(dto);
  vi.spyOn(backend(), "getPage").mockResolvedValue(dto);
  vi.spyOn(backend(), "parseQuery").mockResolvedValue({ query: { anchor: "page", filter: { kind: "and", items: [] }, source: { kind: "builder" } }, view: {} });
  const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(result(["Tagged page"]));
  applySidebarSession({ right: true, items: [{ kind: "page", name: "Research", pageKind: "page" }] });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <RightSidebar />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".tagged-pages li")?.textContent).toBe("Tagged page"));
    const fold = host.querySelector<HTMLButtonElement>(".tagged-pages button")!;
    fold.click(); expect(host.querySelector(".tagged-pages li")).toBeNull();
    expect(fold.getAttribute("aria-expanded")).toBe("false");
    fold.click(); expect(host.querySelector(".tagged-pages li")).not.toBeNull();
    run.mockResolvedValue(result([])); bumpDataRev();
    await vi.waitFor(() => expect(host.querySelector(".tagged-pages")).toBeNull());
  } finally { dispose(); }
});
it("a failed tag query displays a retry instead of claiming no tagged pages", async () => {
  vi.spyOn(backend(), "parseQuery").mockRejectedValueOnce(new Error("read failed"))
    .mockResolvedValue({ query: { anchor: "page", filter: { kind: "and", items: [] }, source: { kind: "builder" } }, view: {} });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(result(["Recovered page"]));
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <TaggedPages name="Research" />, host);
  try {
    await vi.waitFor(() => expect(host.textContent).toContain("Couldn’t load tagged pages."));
    host.querySelector<HTMLButtonElement>(".resource-failure-retry")!.click();
    await vi.waitFor(() => expect(host.querySelector(".tagged-pages li")?.textContent).toBe("Recovered page"));
  } finally { dispose(); }
});
