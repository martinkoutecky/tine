import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { createSignal, For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ensurePageLoaded, resetStore } from "../document";
import { resetNearObserverForTests } from "../lazyObserve";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";
import { queueReferenceCommit, queueReferenceRead } from "../referenceAdmission";
import { initParser } from "../render/parse";
import type { BlockDto, PageRead } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";
import { QueryGroup } from "./QueryGroup";

beforeAll(initParser);
afterEach(() => {
  resetNearObserverForTests();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

function observeViewportForTest() {
  const observers: { targets: Set<Element>; rootMargin?: string; intersect: IntersectionObserverCallback }[] = [];
  let detached = 0;
  vi.stubGlobal("IntersectionObserver", class {
    targets = new Set<Element>();
    constructor(intersect: IntersectionObserverCallback, options?: IntersectionObserverInit) {
      observers.push({ targets: this.targets, rootMargin: options?.rootMargin, intersect });
    }
    observe(element: Element) { this.targets.add(element); if (!element.isConnected) detached += 1; }
    unobserve(element: Element) { this.targets.delete(element); }
    disconnect() { this.targets.clear(); }
  });
  const deliver = (observer: typeof observers[number], targets: Element[], isIntersecting: boolean) => {
    const entries = targets.filter(target => observer.targets.has(target)).map(target => ({ target, isIntersecting }) as IntersectionObserverEntry);
    if (entries.length) observer.intersect(entries, {} as IntersectionObserver);
  };
  return {
    get observed() { return new Set(observers.flatMap(observer => [...observer.targets])); },
    get detached() { return detached; },
    visibility: (isIntersecting = true) => [...observers].forEach(observer => deliver(observer, [...observer.targets], isIntersecting)),
    intersect: (margin: string, targets: Element[], isIntersecting = true) => {
      for (const observer of [...observers]) if (observer.rootMargin === margin) deliver(observer, targets, isIntersecting);
    },
  };
}

describe("I-25 result groups defer membership work until visible", () => {
  it.each(["query", "ref", "embed"] as const)("keeps offscreen %s membership dormant and hydrates the latest rows", async (surface) => {
    const viewport = observeViewportForTest();
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
      expect(viewport.observed.size).toBe(1);
      expect(idReads, "I-25: an offscreen LiveRefGroup must not build row maps, root sets or resources").toBe(0);
      expect(read).not.toHaveBeenCalled();
      setBlocks([row("latest-result", "Latest result")]);
      expect(idReads).toBe(0);
      viewport.visibility();
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
      await queueReferenceCommit(() => {}, () => true);
      expect(read).toHaveBeenCalledTimes(1);
    } finally {
      dispose();
      complete.forEach(finish => finish());
    }
  });

  it("retries once when a query group re-entered the near margin during a read that found no page", async () => {
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
      complete.shift()!();
      await expect.poll(() => read.mock.calls.length).toBe(2);
      complete.shift()!();
      await queueReferenceCommit(() => {}, () => true);
      expect(read).toHaveBeenCalledTimes(2);
    } finally {
      dispose();
      complete.forEach(finish => finish());
    }
  });

  it.each(["query", "embed"] as const)("a %s group's queued source read watches its host only once the host is attached", async surface => {
    const viewport = observeViewportForTest();
    const complete: (() => void)[] = [];
    const read = vi.spyOn(backend(), "getPage").mockImplementation(() => new Promise(resolve => complete.push(() => resolve(null))));
    const host = document.createElement("div");
    document.body.append(host);
    const group = { page: "Source", kind: "page" as const, blocks: [{ id: "hit", raw: "Result", children: [], collapsed: false }] };
    const dispose = render(() => surface === "query" ? <LiveRefGroup {...group} surface="query" eager /> : <LiveRefGroup {...group} surface="embed" />, host);
    try {
      viewport.visibility();
      await expect.poll(() => read.mock.calls.length).toBe(1);
      expect(viewport.detached).toBe(0);
    } finally {
      dispose();
      complete.forEach(finish => finish());
    }
  });
});

const result = (i: number): BlockDto => ({ id: `hit-${i}`, raw: `Result ${i}`, children: [], collapsed: false });
const source = (i: number): PageRead => ({
  id: `pages/Source ${i}.md`, name: `Source ${i}`, title: `Source ${i}`, kind: "page", format: "md", pre_block: null,
  blocks: [{ ...result(i), children: [{ ...result(i), id: `child-${i}`, raw: `Child ${i}` }] }],
});

it.each(["query", "block embed", "page embed"] as const)("reads and renders a visible %s ahead of twelve departed groups", async surface => {
  vi.useFakeTimers();
  const { intersect } = observeViewportForTest();
  const host = document.createElement("div");
  document.body.append(host);
  const groups = Array.from({ length: 13 }, (_, i) => ({ page: source(i).name, kind: "page" as const, blocks: [result(i)] }));
  if (surface === "query") adoptResults(groups.flatMap(group => group.blocks), captureResultsTicket());
  const pending = new Map<string, (dto: PageRead | null) => void>();
  const read = vi.spyOn(backend(), "getPage").mockImplementation(name => new Promise(resolve => pending.set(name, resolve)));
  const dispose = render(() => <For each={groups}>{group => surface === "query" ? <QueryGroup group={() => group} />
    : <LiveRefGroup {...group} surface="embed" wholePage={surface === "page embed"} embedId={surface === "block embed" ? group.blocks[0].id : undefined} />
  }</For>, host);
  let releaseReads!: () => void;
  const heldRead = new Promise<void>(resolve => { releaseReads = resolve; });
  const held = Array.from({ length: 2 }, () => queueReferenceRead(() => heldRead, () => true, { urgent: () => true }));
  const mounted: Element[] = [];
  const mounts = new MutationObserver(records => {
    for (const { target } of records) {
      if (target instanceof Element && target.matches('.live-ref-group[data-find-source-mounted="true"]')) mounted.push(target);
    }
  });
  mounts.observe(host, { subtree: true, attributes: true, attributeFilter: ["data-find-source-mounted"] });
  try {
    await vi.runAllTimersAsync();
    const inner = () => [...host.querySelectorAll(".live-ref-group")];
    const shells = surface === "query" ? [...host.querySelectorAll(".query-group")] : inner();
    intersect("1200px 0px", shells.slice(0, 12));
    intersect("0px 0px", inner().slice(0, 12));
    intersect("1200px 0px", shells.slice(0, 12), false);
    intersect("0px 0px", inner().slice(0, 12), false);
    intersect("1200px 0px", shells.slice(12));
    const visible = inner()[12];
    intersect("0px 0px", [visible]);
    expect(read).not.toHaveBeenCalled();
    releaseReads();
    await vi.runAllTimersAsync();
    expect(read.mock.calls.map(([name]) => name)).toEqual(["Source 12", "Source 0"]);

    // Both replies queue offscreen. The target returns to view before either commit.
    intersect("0px 0px", [visible], false);
    pending.get("Source 0")!(source(0));
    pending.get("Source 12")!(source(12));
    await vi.advanceTimersByTimeAsync(0);
    expect(mounted).toHaveLength(0);
    intersect("0px 0px", [visible]);
    await vi.runAllTimersAsync();
    expect(mounted[0]).toBe(visible);
    expect(visible.querySelector('[data-block-id="child-12"]')?.textContent).toContain("Child 12");
  } finally {
    mounts.disconnect();
    dispose();
    releaseReads();
    pending.forEach(finish => finish(null));
    await vi.runAllTimersAsync();
    await Promise.all(held);
  }
});

it("uses the source already installed while an occurrence waits for a read slot", async () => {
  vi.useFakeTimers();
  const { intersect } = observeViewportForTest();
  const host = document.createElement("div");
  document.body.append(host);
  const read = vi.spyOn(backend(), "getPage").mockResolvedValue(source(0));
  const dispose = render(() => <LiveRefGroup page="Source 0" kind="page" blocks={[result(0)]} surface="embed" />, host);
  try {
    const group = host.querySelector(".live-ref-group")!;
    intersect("1200px 0px", [group]);
    expect(read).not.toHaveBeenCalled();
    ensurePageLoaded(source(0));
    await vi.runAllTimersAsync();
    expect(read).not.toHaveBeenCalled();
    expect(group.querySelector('[data-block-id="child-0"]')?.textContent).toContain("Child 0");
  } finally { dispose(); }
});
