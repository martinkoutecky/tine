import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { resetNearObserverForTests } from "../lazyObserve";
import { PageView } from "./Page";

class OffscreenObserver {
  static current: OffscreenObserver;
  targets = new Set<Element>();
  constructor(private callback: IntersectionObserverCallback) { OffscreenObserver.current = this; }
  observe(target: Element) { this.targets.add(target); }
  unobserve(target: Element) { this.targets.delete(target); }
  disconnect() {}
  revealFirst() { this.callback([{ target: [...this.targets][0], isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver); }
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetNearObserverForTests();
  resetStore();
  resetTabsToJournals();
  document.body.replaceChildren();
});

it("I-25: routed large-page opening keeps offscreen bodies unparsed when native facets report no properties", async () => {
  vi.stubGlobal("IntersectionObserver", OffscreenObserver);
  const count = 500;
  const blocks = Array.from({ length: count }, (_, i) => ({
    id: `qc6-${i}`, has_id: false, raw: `QC6 **unique body ${i}**`,
    children: [], collapsed: false,
  }));
  vi.spyOn(backend(), "getPage").mockResolvedValue({
    id: "pages/QC6.md", name: "QC6", title: "QC6", kind: "page", pre_block: null, blocks,
  });
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "QC6", pageKind: "page" });
  const host = document.createElement("div"); document.body.append(host);
  const stats = { calls: 0, hits: 0, misses: 0 };
  (window as unknown as { __tineParseStats: typeof stats }).__tineParseStats = stats;
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.querySelectorAll(".ls-block")).toHaveLength(count));
    expect(host.querySelectorAll(".ast-deferred")).toHaveLength(count);
    expect(host.querySelector(".ast-deferred")?.textContent).toBe(blocks[0].raw);
    expect(stats.misses, "I-25: property chrome must reuse native negative facets; see render/facets.ts").toBe(0);
    OffscreenObserver.current.revealFirst();
    expect(host.querySelector("strong")?.textContent).toBe("unique body 0");
    expect(stats.misses).toBe(1);
  } finally { dispose(); }
});
