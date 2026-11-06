import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { resetNearObserverForTests } from "../lazyObserve";
import { initParser } from "../render/parse";
import type { BlockDto, RefGroup } from "../types";
import { QueryGroup, QueryGroups } from "./QueryGroup";

// og query-sheet journey step 6 (2026-10-03): after an app restart a query block showed its result count and its
// "Tasks" group header but not the "TODO alpha task" row. The DOM dump of the failing run had
// `<div class="query-group"><div class="query-page">Tasks</div><div class="live-ref-group" style="min-height:1.9em">`:
// the group's header gate had fired and the row gate (a second, independent IntersectionObserver registration on the
// inner `.live-ref-group`) had not. Invariant: a group header never renders without its rows.

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

describe("a query group header never renders without its rows", () => {
  it("reserves the complete previously mounted query shell, including its header (GH #810)", async () => {
    let intersect!: IntersectionObserverCallback;
    vi.stubGlobal("IntersectionObserver", class {
      constructor(cb: IntersectionObserverCallback) { intersect = cb; }
      observe() {}
      unobserve() {}
      disconnect() {}
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({ height: 340 } as DOMRect);
    const root = document.createElement("div");
    document.body.append(root);
    const group: RefGroup = { page: "Tasks", kind: "page", blocks: [{ id: "alpha", raw: "TODO alpha", collapsed: false, children: [] }] };
    const mount = () => render(() => <QueryGroup hostBlockId="query-owner" group={() => group} />, root);
    let dispose = mount();
    const outer = root.querySelector(".query-group")!;
    intersect([{ target: outer, isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
    await expect.poll(() => root.textContent).toContain("alpha");
    dispose();
    dispose = mount();
    try {
      expect(root.querySelector<HTMLElement>(".query-group")!.style.minHeight).toBe("340px");
      expect(root.textContent).toBe("");
    } finally { dispose(); }
  });
  it("mounts the rows with the header when only the group's own gate fires", async () => {
    let intersect!: IntersectionObserverCallback;
    const observed = new Set<Element>();
    vi.stubGlobal("IntersectionObserver", class {
      constructor(callback: IntersectionObserverCallback) { intersect = callback; }
      observe(element: Element) { observed.add(element); }
      unobserve(element: Element) { observed.delete(element); }
      disconnect() { observed.clear(); }
    });
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    const block: BlockDto = { id: "alpha", raw: "TODO alpha task", collapsed: false, children: [], breadcrumb: [] };
    const group = { page: "Tasks", kind: "page", blocks: [block] } as RefGroup;
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <QueryGroups groups={() => new Map([["Tasks", group]])} />, root);
    try {
      await expect.poll(() => root.querySelector(".query-group")).not.toBeNull();
      const outer = root.querySelector(".query-group")!;
      expect(observed.has(outer)).toBe(true);
      // Only the outer gate reports "near"; the observer never reports anything for any other element.
      intersect([{ target: outer, isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
      await expect.poll(() => root.querySelector(".query-page")?.textContent).toBe("Tasks");
      await expect.poll(() => root.textContent, { timeout: 2000 }).toContain("alpha task");
    } finally { dispose(); }
  });
});
