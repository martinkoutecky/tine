import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { refreshPageIndex, resetPageIndex } from "../pageIndex";
import { paneRouter, resetPaneLayoutToSingle } from "../panes";
import { InlineText } from "./inline";
import { itemKind } from "../favoritesLayout";
import type { PageInventoryEntry } from "../types";
import { PageView } from "../components/Page";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";

afterEach(() => { resetPageIndex(); resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });

it.each([
  ["Oct 26th, 2026", false, "page"],
  ["2026-10-26", false, "page"],
  ["Oct 26th, 2026", true, "journal"],
  ["2026-10-26", true, "journal"],
] as const)("clicking [[%s]] on a routed page (journal exists: %s) opens %s", async (name, journal, expected) => {
  const entries: PageInventoryEntry[] = [{
    key: name.toLowerCase(), name, is_journal: false, day: null,
    target: { kind: "existing", id: "pages/imported.md", others: [] },
  }];
  if (journal) entries.push({
    key: "oct 26th, 2026", name: "Oct 26th, 2026", is_journal: true, day: 20261026,
    target: { kind: "existing", id: "journals/2026_10_26.md", others: [] },
  });
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries });
  await refreshPageIndex();
  resetPaneLayoutToSingle({ tabs: [{ history: [{ kind: "page", name: "Source", pageKind: "page" }], pos: 0, pinned: false }], activeIndex: 0 });
  const router = paneRouter("main");
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <InlineText text={`[[${name}]]`} />, host);
  try {
    host.querySelector<HTMLElement>(".page-ref")!.click();
    expect(router.route()).toMatchObject({ kind: "page", name, pageKind: expected });
    expect(host.querySelector(".page-ref")!.hasAttribute("data-missing-page")).toBe(false);
    expect(itemKind(name)).toBe(expected);
  } finally { dispose(); }
});

it("a link in a loaded routed page displays the existing date page's content", async () => {
  const name = "Oct 26th, 2026";
  const source = { id: "pages/Source.md", name: "Source", title: "Source", kind: "page" as const, pre_block: null,
    blocks: [{ id: "source-link", raw: `[[${name}]]`, collapsed: false, children: [] }] };
  const target = { ...source, id: "pages/imported.md", name, title: name,
    blocks: [{ id: "target-content", raw: "Imported date page content", collapsed: false, children: [] }] };
  vi.spyOn(backend(), "pageInventory").mockResolvedValue({ rev: "1", entries: [
    { key: name.toLowerCase(), name, is_journal: false, day: null, target: { kind: "existing", id: target.id, others: [] } },
  ] });
  await refreshPageIndex();
  vi.spyOn(backend(), "getPage").mockImplementation(async (requested, kind) => requested === "Source" ? source : kind === "page" ? target : null);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => path === source.id ? source : target);
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
    expect(paneRouter("main").route()).toMatchObject({ name, pageKind: "page" });
  } finally { dispose(); }
});
