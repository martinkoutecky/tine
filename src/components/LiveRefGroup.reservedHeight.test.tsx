import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { resetNearObserverForTests } from "../lazyObserve";
import { initParser } from "../render/parse";
import type { BlockDto } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";

// GH #810: scrolling a page with an expanded block embed jumped. An outline window that scrolled out of view keeps a
// spacer of its measured height; coming back, it remounted with the embed's group unmounted behind a one-row
// placeholder (1.9em), so the window was one embed shorter than its spacer and every block below it moved, and moved
// back when the rows mounted. Invariant: a group that was mounted reserves the height it had when it unmounted.
// (jsdom has no layout; the rect is stubbed, the real geometry is scripts/e2e-gh810-embed-scroll.mjs.)

beforeAll(initParser);
beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class {
    constructor(private callback: ResizeObserverCallback) {}
    observe(target: Element) {
      queueMicrotask(() => this.callback([{ target, borderBoxSize: [{ blockSize: target.getBoundingClientRect().height }] } as unknown as ResizeObserverEntry], this as unknown as ResizeObserver));
    }
    unobserve() {}
    disconnect() {}
  });
});
afterEach(() => {
  resetNearObserverForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

const child = (n: number): BlockDto => ({ id: `gh810-child-${n}`, raw: `child ${n}`, collapsed: false, children: [], breadcrumb: [] });
const source = (id: string): BlockDto => ({ id, raw: "embedded root", collapsed: false, children: Array.from({ length: 8 }, (_, n) => child(n)), breadcrumb: [] });

function stubObserver() {
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
    near: (target: Element) => intersect([{ target, isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver),
  };
}

function mount(root: HTMLElement, blockId: string, host: string) {
  return render(() => <LiveRefGroup page="Embed source" kind="page" blocks={[source(blockId)]} embedId={blockId}
    hostBlockId={host} surface="embed" />, root);
}

describe("a live ref group reserves the height it had when it last unmounted (GH #810)", () => {
  it("comes back with its rows' height as the placeholder, not one row's estimate", async () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      const height = this.classList.contains("live-ref-group") ? 288 : 0;
      return { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: height, width: 0, height, toJSON: () => ({}) } as DOMRect;
    });
    const io = stubObserver();
    const root = document.createElement("div");
    document.body.appendChild(root);

    // First sight: one-row estimate, then the rows mount when the group is near.
    let dispose = mount(root, "gh810-source", "gh810-host");
    const first = root.querySelector<HTMLElement>(".live-ref-group")!;
    expect(first.style.minHeight).toBe("1.9em");
    expect(io.observed.has(first)).toBe(true);
    io.near(first);
    await expect.poll(() => root.textContent).toContain("embedded root");
    expect(first.style.minHeight).toBe("");
    dispose();

    // Its window scrolls back into view: the same embed is mounted again, not near yet.
    dispose = mount(root, "gh810-source", "gh810-host");
    try {
      const again = root.querySelector<HTMLElement>(".live-ref-group")!;
      expect(again.style.minHeight).toBe("288px");
      expect(again.textContent).toBe("");
    } finally { dispose(); }

    // Another embed that has never been mounted keeps the estimate.
    dispose = mount(root, "gh810-other", "gh810-other-host");
    try {
      expect(root.querySelector<HTMLElement>(".live-ref-group")!.style.minHeight).toBe("1.9em");
    } finally { dispose(); }
  });

  it("does not reserve the height of a group that never mounted its rows", () => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      const height = this.classList.contains("live-ref-group") ? 31 : 0;
      return { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: height, width: 0, height, toJSON: () => ({}) } as DOMRect;
    });
    stubObserver();
    const root = document.createElement("div");
    document.body.appendChild(root);
    let dispose = mount(root, "gh810-unseen", "gh810-unseen-host");
    dispose();
    dispose = mount(root, "gh810-unseen", "gh810-unseen-host");
    try {
      expect(root.querySelector<HTMLElement>(".live-ref-group")!.style.minHeight).toBe("1.9em");
    } finally { dispose(); }
  });

  it.each(["query", "ref"] as const)("reserves %s groups by owner identity without inspecting a row", async (surface) => {
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      const height = this.classList.contains("live-ref-group") ? 288 : 0;
      return { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: height, width: 0, height, toJSON: () => ({}) } as DOMRect;
    });
    const io = stubObserver();
    const root = document.createElement("div");
    document.body.appendChild(root);
    const query = () => render(() => <LiveRefGroup page="Tasks" kind="page" blocks={[source("gh810-query-row")]}
      surface={surface} heightKey="gh810-result-owner" />, root);
    let dispose = query();
    io.near(root.querySelector<HTMLElement>(".live-ref-group")!);
    await expect.poll(() => root.textContent).toContain("embedded root");
    dispose();
    dispose = query();
    try {
      expect(root.querySelector<HTMLElement>(".live-ref-group")!.style.minHeight).toBe("288px");
    } finally { dispose(); }
  });
});
