import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
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
  static approach(selector: string) {
    for (const observer of [...Viewport.observers]) {
      const entries = [...observer.targets].filter(target => target.matches(selector))
        .map(target => ({ target, isIntersecting: true, boundingClientRect: target.getBoundingClientRect() } as IntersectionObserverEntry));
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

it("counts an in-outline embed and query before scrolling their groups into view (GH #935)", async () => {
  vi.stubGlobal("IntersectionObserver", Viewport);
  Range.prototype.getClientRects = vi.fn(() => [] as unknown as DOMRectList);
  vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => {});
  const embedded = block("embed-hit", "needle embedded");
  const queried = block("query-hit", "TODO needle queried");
  const pages = new Map([
    ["Host", page("Host", [block("body", "needle own content"), block("embed-host", "{{embed [[Embedded]]}}"), block("query-host", "{{query (task TODO)}}")])],
    ["Embedded", page("Embedded", [embedded])], ["Tasks", page("Tasks", [queried])],
  ]);
  vi.spyOn(backend(), "getPage").mockImplementation(async name => pages.get(name) ?? null);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => [...pages.values()].find(page => page.id === path) ?? null);
  backendReadsQueries({ "(task TODO)": { form: "(task TODO)" } });
  vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([{ page: "Tasks", kind: "page", blocks: [queried] }]));
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Host", pageKind: "page" });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <><div data-pane-id="main"><PageView /></div><InPageFind /></>, host);
  try {
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="embed-host"]')).not.toBeNull());
    // The macro hosts are on the page, but their result groups are below the viewport.
    Viewport.approach(".ast-deferred");
    await vi.waitFor(() => expect(host.querySelector(".embed-block .live-ref-group")).not.toBeNull());
    await vi.waitFor(() => expect(host.querySelector(".query-group")).not.toBeNull());
    expect(host.querySelector('[data-block-id="embed-hit"]')).toBeNull();
    expect(host.querySelector('[data-block-id="query-hit"]')).toBeNull();
    openInPageFind(); setInPageFindQuery("needle");
    const before = inPageFindMatches().length;
    Viewport.approach(".live-ref-group,.query-group");
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="embed-hit"]')).not.toBeNull());
    await vi.waitFor(() => expect(host.querySelector('[data-block-id="query-hit"]')).not.toBeNull());
    Viewport.approach(".ast-deferred");
    // Existing DOM rows are re-read only on another query/surface revision.
    setInPageFindQuery("needl"); setInPageFindQuery("needle");
    const after = inPageFindMatches().length;
    console.info(JSON.stringify({ journey: "routed Host Find", beforeScroll: before, afterScroll: after, expected: 3 }));
    expect({ before, after }).toEqual({ before: 3, after: 3 });
  } finally { dispose(); }
});
