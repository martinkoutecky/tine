import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import { paneRouter, resetPaneLayoutToSingle } from "../panes";
import { InlineText, PageRef } from "./inline";
import { itemKind } from "../favoritesLayout";
import type { PageInventoryEntry } from "../types";
import { PageView } from "../components/Page";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { setGraphMeta } from "../graphSession";
import type { GraphMeta } from "../types";

afterEach(() => { setGraphMeta(null); resetPageIndex(); resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });

it("GH #959: inline and list page references share bracket visibility while aliases and tags keep their labels", () => {
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <><PageRef name="Target" /><PageRef name="Target" alias="Custom label" /><PageRef name="Target" tag /></>, host);
  try {
    const labels = () => [...host.querySelectorAll("a")].map(link => link.textContent);
    setGraphMeta({ show_brackets: true } as GraphMeta);
    expect(labels()).toEqual(["[[Target]]", "Custom label", "#Target"]);
    setGraphMeta({ show_brackets: false } as GraphMeta);
    expect(labels()).toEqual(["Target", "Custom label", "#Target"]);
  } finally { dispose(); }
});

it.each([
  ["Oct 26th, 2026", "pages/imported.md"],
  ["2026-10-26", "pages/imported.md"],
  ["Oct 26th, 2026", "journals/2026_10_26.md"],
  ["2026-10-26", "journals/2026_10_26.md"],
] as const)("clicking [[%s]] opens the journal held by %s", async (name, id) => {
  const title = "Oct 26th, 2026";
  const entries: PageInventoryEntry[] = [{
    key: title.toLowerCase(), name: title, is_journal: true, day: 20261026,
    target: { kind: "existing", id, others: [] }, link: { name: title, kind: "journal" },
  }];
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries });
  await refreshPageIndex();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Source", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
  const router = paneRouter("main");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <InlineText text={`[[${name}]]`} />, host);
  try {
    host.querySelector<HTMLElement>(".page-ref")!.click();
    expect(router.route()).toMatchObject({ kind: "page", name: title, pageKind: "journal" });
    expect(host.querySelector(".page-ref")!.hasAttribute("data-missing-page")).toBe(false);
    expect(itemKind(name)).toBe("journal");
  } finally { dispose(); }
});

it("a link in a loaded page displays the journal held by a file in pages/", async () => {
  const name = "Oct 26th, 2026";
  const source = { id: "pages/Source.md", name: "Source", title: "Source", kind: "page" as const, pre_block: null,
    blocks: [{ id: "source-link", raw: `[[${name}]]`, collapsed: false, children: [] }] };
  const target = { ...source, id: "pages/imported.md", name, title: name, kind: "journal" as const,
    blocks: [{ id: "target-content", raw: "Imported date page content", collapsed: false, children: [] }] };
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [
    { key: name.toLowerCase(), name, is_journal: true, day: 20261026, target: { kind: "existing", id: target.id, others: [] }, link: { name, kind: "journal" } },
  ] });
  await refreshPageIndex();
  vi.spyOn(backend(), "getPage").mockImplementation(async (requested, kind) => requested === "Source" ? source : requested === name && kind === "journal" ? target : null);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => path === source.id ? source : path === target.id ? target : null);
  vi.spyOn(backend(), "taggedPages").mockResolvedValue({ anchor: "page", pages: [], total: 0, exceeded: false, diagnostics: [], report: { supported: true, ran: [], ignored: [] } });
  loadSingle(source);
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Source", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.textContent).toContain(name));
    const link = host.querySelector<HTMLElement>(".page-ref")!;
    const missing = link.hasAttribute("data-missing-page");
    link.click();
    await vi.waitFor(() => expect(host.textContent).toContain("Imported date page content"));
    expect(missing).toBe(false);
    expect(paneRouter("main").route()).toMatchObject({ name, pageKind: "journal" });
  } finally { dispose(); }
});
