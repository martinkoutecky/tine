import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ensurePageLoaded, pageByName, resetStore } from "../document";
import { closeInPageFind, inPageFindMatches, openInPageFind, setInPageFindQuery } from "../inpageFind";
import { resetNearObserverForTests } from "../lazyObserve";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { initParser } from "../render/parse";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { backendReadsQueries, blockRunResult } from "../tests/queryReadingsTestkit";
import type { BlockDto, PageRead } from "../types";
import { InPageFind } from "./InPageFind";
import { PageView } from "./Page";

// A real viewport admission without jsdom's eager fallback. Only the selected
// hosts approach the viewport; source groups stay distant until explicitly scrolled.
class Viewport {
  static observers: Viewport[] = [];
  targets = new Set<Element>();
  constructor(private callback: IntersectionObserverCallback) { Viewport.observers.push(this); }
  observe(target: Element) { this.targets.add(target); }
  unobserve(target: Element) { this.targets.delete(target); }
  disconnect() { this.targets.clear(); }
  static approach(selector: string, limit = Infinity, near = true) {
    for (const observer of [...Viewport.observers]) {
      const entries = [...observer.targets].filter(target => target.matches(selector)).slice(0, limit)
        .map(target => ({ target, isIntersecting: near, boundingClientRect: target.getBoundingClientRect() } as IntersectionObserverEntry));
      if (entries.length) observer.callback(entries, observer as unknown as IntersectionObserver);
    }
  }
}
const block = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const page = (name: string, blocks: BlockDto[]): PageRead => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", format: "md", pre_block: null, blocks,
});
beforeAll(initParser);
afterEach(() => {
  closeInPageFind({ restoreFocus: false }); setInPageFindQuery("");
  resetNearObserverForTests(); resetSharedQueryResultsForTests(); resetStore(); resetTabsToJournals();
  Viewport.observers = []; vi.unstubAllGlobals(); vi.restoreAllMocks(); document.body.replaceChildren();
});

it.each([
  { preceding: 0, groupCount: 2, sourceRows: 0, collapsed: false }, { preceding: 96, groupCount: 1, sourceRows: 0, collapsed: false },
  { preceding: 0, groupCount: 0, sourceRows: 0, collapsed: true }, { preceding: 0, groupCount: 0, sourceRows: 96, collapsed: false },
])("discloses pending hosts on routed pages: %j (GH #935)", async ({ preceding, groupCount, sourceRows, collapsed }) => {
  vi.stubGlobal("IntersectionObserver", Viewport);
  Range.prototype.getClientRects = vi.fn(() => [] as unknown as DOMRectList);
  vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => {});
  const embedded = block("embed-hit", "needle embedded");
  const queried = block("query-hit", "TODO needle queried");
  const queryArgument = `(task TODO)${collapsed ? " {:collapsed? true}" : ""}`;
  const pages = new Map([
    ["Host", page("Host", [block("body", "needle own content"), ...Array.from({ length: preceding }, (_, i) => block(`filler-${i}`, "ordinary content")), block("embed-host", "{{embed [[Embedded]]}}"), block("query-host", `{{query ${queryArgument}}}`)])],
    ["Embedded", page("Embedded", [embedded, ...Array.from({ length: sourceRows }, (_, i) => block(`source-${i}`, "ordinary content")),
      ...(sourceRows ? [block("embed-tail", "needle tail")] : [])])], ["Tasks", page("Tasks", [queried])],
    ["More", page("More", [block("query-hit-more", "TODO needle more")])],
  ]);
  const reads = vi.spyOn(backend(), "getPage").mockImplementation(async name => pages.get(name) ?? null);
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  const pathReads = vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => [...pages.values()].find(page => page.id === path) ?? null);
  backendReadsQueries({ [queryArgument]: { form: "(task TODO)", opts: collapsed ? "{:collapsed? true}" : "" } });
  const groups = [{ page: "Tasks", kind: "page" as const, blocks: [queried] },
    { page: "More", kind: "page" as const, blocks: pages.get("More")!.blocks }].slice(0, groupCount);
  const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult(groups));
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Host", pageKind: "page" });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <><div data-pane-id="main"><PageView /></div><InPageFind /></>, host);
  const updates: { count: number; note: boolean }[] = [];
  const disclosure = new MutationObserver(() => updates.push({ count: inPageFindMatches().length,
    note: host.querySelector('.inpage-find-bar')?.textContent?.includes("not searched yet") ?? false }));
  try {
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="body"]')).not.toBeNull());
    if (preceding) expect(host.querySelector('[data-block-id="query-host"]')).toBeNull();
    expect(run).not.toHaveBeenCalled();
    const readsBeforeFind = reads.mock.calls.length + pathReads.mock.calls.length;
    openInPageFind(); setInPageFindQuery("needle");
    expect(reads.mock.calls.length + pathReads.mock.calls.length).toBe(readsBeforeFind);
    expect(run).not.toHaveBeenCalled();
    const initial = { count: inPageFindMatches().length, note: host.querySelector('.inpage-find-bar [role="status"]')?.textContent?.match(/\d+[^.]*not searched yet/)?.[0] ?? null };
    console.info(JSON.stringify({ journey: "routed Host Find before scrolling", preceding, groupCount, ...initial }));
    expect(initial.count).toBe(1);
    expect(initial.note).toMatch(/^2\b/);
    disclosure.observe(host.querySelector('.inpage-find-bar')!, { childList: true, subtree: true, characterData: true });
    // Even the macro resource is absent until its outline window and body mount.
    Viewport.approach(".outline-window");
    // The macro hosts are on the page, but their result groups are below the viewport.
    Viewport.approach(".ast-deferred");
    await vi.waitFor(() => expect(host.querySelector(".embed-block .live-ref-group")).not.toBeNull());
    await vi.waitFor(() => expect(host.querySelector('.query-block[data-find-answered="true"]')).not.toBeNull());
    if (groupCount) await vi.waitFor(() => expect(host.querySelector(".query-group")).not.toBeNull());
    expect(host.querySelector('[data-block-id="embed-hit"]')).toBeNull();
    expect(host.querySelector('[data-block-id="query-hit"]')).toBeNull();
    const before = inPageFindMatches().length;
    Viewport.approach(".live-ref-group");
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="embed-hit"]')).not.toBeNull());
    await vi.waitFor(() => expect(host.querySelector('.embed-block .live-ref-group[data-find-source-mounted="true"]')).not.toBeNull());
    Viewport.approach(".ast-deferred");
    if (sourceRows) {
      await vi.waitFor(() => expect(host.querySelector('.inpage-find-bar [role="status"]')?.textContent).toContain("1 embed or query not searched yet"));
      expect(host.querySelector('[data-block-id="embed-tail"]')).toBeNull();
      Viewport.approach(".embed-block .outline-window");
      await vi.waitFor(() => expect(host.querySelector('[data-block-id="embed-tail"]')).not.toBeNull());
      Viewport.approach(".ast-deferred");
    }
    if (groupCount) {
      await vi.waitFor(() => expect(host.querySelector('.inpage-find-bar')?.textContent).toContain("1 embed or query not searched yet"));
      Viewport.approach(".query-group", 1);
      await vi.waitFor(() => expect(host.querySelector('[data-block-id="query-hit"]')).not.toBeNull());
      await vi.waitFor(() => expect(host.querySelector('.query-group .live-ref-group[data-find-source-mounted="true"]')).not.toBeNull());
      Viewport.approach(".ast-deferred");
      if (groupCount > 1) {
        await vi.waitFor(() => expect(inPageFindMatches()).toHaveLength(3));
        expect(host.querySelector('.inpage-find-bar')?.textContent).toContain("1 embed or query not searched yet");
      }
      Viewport.approach(".query-group");
      if (groupCount > 1) await vi.waitFor(() => expect(host.querySelector('[data-block-id="query-hit-more"]')).not.toBeNull());
      await vi.waitFor(() => expect(host.querySelectorAll('.query-group .live-ref-group[data-find-source-mounted="true"]')).toHaveLength(groupCount));
    }
    Viewport.approach(".ast-deferred");
    // Let the real mutation observer refresh Find; the reader does not retype.
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    const after = inPageFindMatches().length;
    const finalNote = host.querySelector('.inpage-find-bar')?.textContent?.includes("not searched yet") ?? false;
    console.info(JSON.stringify({ journey: "routed Host Find", preceding, initial, beforeScroll: before, afterScroll: after, queryRuns: run.mock.calls.length, finalNote }));
    expect({ before, after, finalNote }).toEqual({ before: 1, after: 2 + groupCount + Number(sourceRows > 0), finalNote: false });
    expect(run).toHaveBeenCalledTimes(1);
    expect(updates.every(update => update.note || update.count === after)).toBe(true);
    disclosure.disconnect();
    Viewport.approach(".outline-window,.live-ref-group,.query-group,.ast-deferred");
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    expect(inPageFindMatches()).toHaveLength(after);
    if (sourceRows) {
      Viewport.approach(".live-ref-group", Infinity, false);
      for (let i = 0; i < 81; i++) ensurePageLoaded(page(`Unrelated ${i}`, []));
      expect(pageByName("Embedded")).toBeDefined();
      await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      expect(inPageFindMatches()).toHaveLength(after);
      expect(host.querySelector('.inpage-find-bar')?.textContent).not.toContain("not searched yet");
      closeInPageFind({ restoreFocus: false });
      for (let i = 81; i < 162; i++) ensurePageLoaded(page(`Unrelated ${i}`, []));
      expect(pageByName("Embedded")).toBeUndefined();
      const readsBeforeReopen = reads.mock.calls.length + pathReads.mock.calls.length;
      openInPageFind();
      expect(reads.mock.calls.length + pathReads.mock.calls.length).toBe(readsBeforeReopen);
      expect(host.querySelector('.inpage-find-bar')?.textContent).toContain("1 embed or query not searched yet");
      Viewport.approach(".live-ref-group");
      await vi.waitFor(() => expect(host.querySelector('.embed-block .live-ref-group[data-find-source-mounted="true"]')).not.toBeNull());
      Viewport.approach(".embed-block .outline-window,.ast-deferred");
      await vi.waitFor(() => expect(inPageFindMatches()).toHaveLength(after));
      await vi.waitFor(() => expect(host.querySelector('.inpage-find-bar')?.textContent).not.toContain("not searched yet"));
    }
  } finally { disclosure.disconnect(); dispose(); }
});

// Review HOLD on f471f00d0: an offscreen BEGIN_QUERY container showed no pending note, then silently added matches.
it("counts a BEGIN_QUERY container as a pending host until its results are searched (GH #935)", async () => {
  vi.stubGlobal("IntersectionObserver", Viewport);
  Range.prototype.getClientRects = vi.fn(() => [] as unknown as DOMRectList);
  vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => {});
  const queried = block("query-hit", "TODO needle queried");
  const pages = new Map([
    ["Host", page("Host", [block("body", "needle own content"),
      block("query-host", "#+BEGIN_QUERY\n{:query (task TODO) :table-view? false}\n#+END_QUERY")])],
    ["Tasks", page("Tasks", [queried])],
  ]);
  vi.spyOn(backend(), "getPage").mockImplementation(async name => pages.get(name) ?? null);
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => [...pages.values()].find(page => page.id === path) ?? null);
  backendReadsQueries({ "(task TODO) {:table-view? false}": { form: "(task TODO)", opts: "{:table-view? false}" } });
  const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([{ page: "Tasks", kind: "page", blocks: [queried] }]));
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Host", pageKind: "page" });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <><div data-pane-id="main"><PageView /></div><InPageFind /></>, host);
  try {
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="body"]')).not.toBeNull());
    openInPageFind(); setInPageFindQuery("needle");
    expect(run).not.toHaveBeenCalled();
    expect(inPageFindMatches()).toHaveLength(1);
    expect(host.querySelector('.inpage-find-bar [role="status"]')?.textContent).toContain("1 embed or query not searched yet");
    Viewport.approach(".outline-window,.ast-deferred");
    await vi.waitFor(() => expect(host.querySelector('.query-block[data-find-answered="true"]')).not.toBeNull());
    await vi.waitFor(() => expect(host.querySelector(".query-group")).not.toBeNull());
    Viewport.approach(".query-group,.live-ref-group,.ast-deferred");
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="query-hit"]')).not.toBeNull());
    await vi.waitFor(() => expect(host.querySelector('.query-group .live-ref-group[data-find-source-mounted="true"]')).not.toBeNull());
    Viewport.approach(".ast-deferred");
    await vi.waitFor(() => expect(inPageFindMatches()).toHaveLength(2));
    await vi.waitFor(() => expect(host.querySelector(".inpage-find-bar")?.textContent).not.toContain("embed or query not searched yet"));
  } finally { dispose(); }
});
