import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { resetNearObserverForTests } from "../lazyObserve";
import { initParser } from "../render/parse";
import type { BlockDto } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";

beforeAll(initParser);
afterEach(() => {
  resetNearObserverForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

describe("I-25 result groups defer membership work until visible", () => {
  it.each(["query", "ref", "embed"] as const)("keeps offscreen %s membership dormant and hydrates the latest rows", async (surface) => {
    let intersect!: IntersectionObserverCallback;
    const observed = new Set<Element>();
    vi.stubGlobal("IntersectionObserver", class {
      constructor(callback: IntersectionObserverCallback) { intersect = callback; }
      observe(element: Element) { observed.add(element); }
      unobserve(element: Element) { observed.delete(element); }
      disconnect() { observed.clear(); }
    });
    let idReads = 0;
    const row = (id: string, raw: string): BlockDto => ({
      get id() { idReads += 1; return id; }, raw, children: [], collapsed: false,
    });
    const [blocks, setBlocks] = createSignal([row("old-result", "Old result")]);
    const read = vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LiveRefGroup page="Source" kind="page" blocks={blocks()} surface={surface} />, root);
    try {
      expect(observed.size).toBe(1);
      expect(idReads, "I-25: an offscreen LiveRefGroup must not build row maps, root sets or resources").toBe(0);
      expect(read).not.toHaveBeenCalled();
      setBlocks([row("latest-result", "Latest result")]);
      expect(idReads).toBe(0);
      const target = [...observed][0];
      intersect([{ target, isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver);
      await expect.poll(() => root.textContent).toContain("Latest result");
      expect(root.textContent).not.toContain("Old result");
      expect(read).toHaveBeenCalledTimes(1);
      expect(idReads).toBeGreaterThan(0);
    } finally { dispose(); }
  });
});
