import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { createSignal, For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { resetNearObserverForTests } from "../lazyObserve";
import { queueReferenceCommit } from "../referenceAdmission";
import { initParser } from "../render/parse";
import type { BlockDto } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";
import { QueryGroup } from "./QueryGroup";

beforeAll(initParser);
afterEach(() => {
  resetNearObserverForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

function observeViewportForTest() {
  let intersect!: IntersectionObserverCallback;
  const observed = new Set<Element>();
  vi.stubGlobal("IntersectionObserver", class {
    constructor(callback: IntersectionObserverCallback) { intersect = callback; }
    observe(element: Element) { observed.add(element); }
    unobserve(element: Element) { observed.delete(element); }
    disconnect() { observed.clear(); }
  });
  return {
    observed,
    visibility: (isIntersecting = true) => intersect([...observed].map(target => ({ target, isIntersecting }) as IntersectionObserverEntry), {} as IntersectionObserver),
  };
}

describe("I-25 result groups defer membership work until visible", () => {
  it.each(["query", "ref", "embed"] as const)("keeps offscreen %s membership dormant and hydrates the latest rows", async (surface) => {
    const { observed, visibility } = observeViewportForTest();
    let idReads = 0;
    const row = (id: string, raw: string): BlockDto => ({
      get id() { idReads += 1; return id; }, raw, children: [], collapsed: false,
    });
    const [blocks, setBlocks] = createSignal([row("old-result", "Old result")]);
    const read = vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LiveRefGroup page="Source" kind="page" blocks={blocks()} surface={surface}
      heightKey="result-owner" hostBlockId="host" />, root);
    try {
      expect(observed.size).toBe(1);
      expect(idReads, "I-25: an offscreen LiveRefGroup must not build row maps, root sets or resources").toBe(0);
      expect(read).not.toHaveBeenCalled();
      setBlocks([row("latest-result", "Latest result")]);
      expect(idReads).toBe(0);
      visibility();
      await expect.poll(() => root.textContent).toContain("Latest result");
      expect(root.textContent).not.toContain("Old result");
      await expect.poll(() => read.mock.calls.length).toBe(1);
      expect(idReads).toBeGreaterThan(0);
    } finally { dispose(); }
  });

  it.each(["query", "embed"] as const)("bounds source reads when several %s groups approach together", async (surface) => {
    const { visibility } = observeViewportForTest();
    const complete: (() => void)[] = [];
    const read = vi.spyOn(backend(), "getPage").mockImplementation(() => new Promise(resolve => complete.push(() => resolve(null))));
    const host = document.createElement("div");
    document.body.append(host);
    const indices = Array.from({ length: 8 }, (_, i) => i);
    const dispose = render(() => <For each={indices}>{i => {
      const group = { page: `Source ${i}`, kind: "page" as const,
        blocks: [{ id: `hit-${i}`, raw: `Result ${i}`, children: [], collapsed: false }] };
      return surface === "query" ? <QueryGroup group={() => group} />
        : <LiveRefGroup {...group} surface={surface} />;
    }}</For>, host);
    try {
      visibility();
      await vi.waitFor(() => expect(read.mock.calls.length).toBeGreaterThanOrEqual(2));
      expect(read).toHaveBeenCalledTimes(2);
      expect(host.querySelectorAll(".ls-block")).toHaveLength(indices.length);
    } finally {
      dispose();
      complete.forEach(finish => finish());
    }
  });

  it("keeps the pending source read when a query group crosses the near margin again", async () => {
    const { visibility } = observeViewportForTest();
    const complete: (() => void)[] = [];
    const read = vi.spyOn(backend(), "getPage").mockImplementation(() => new Promise(resolve => complete.push(() => resolve(null))));
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <LiveRefGroup surface="query" page="Source" kind="page"
      blocks={[{ id: "hit", raw: "Result", children: [], collapsed: false }]} />, host);
    try {
      visibility();
      await expect.poll(() => read.mock.calls.length).toBe(1);
      visibility(false);
      visibility();
      await queueReferenceCommit(() => {}, () => true, () => 1);
      expect(read).toHaveBeenCalledTimes(1);
    } finally {
      dispose();
      complete.forEach(finish => finish());
    }
  });
});
