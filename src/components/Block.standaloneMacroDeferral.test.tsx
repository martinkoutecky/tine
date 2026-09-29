import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetNearObserverForTests } from "../lazyObserve";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { Block } from "./Block";

class ManualNearObserver implements IntersectionObserver {
  static instance: ManualNearObserver | null = null;
  readonly root = null;
  readonly rootMargin = "1200px 0px";
  readonly thresholds = [0];
  private targets = new Set<Element>();

  constructor(private readonly callback: IntersectionObserverCallback) {
    ManualNearObserver.instance = this;
  }
  observe(target: Element): void { this.targets.add(target); }
  unobserve(target: Element): void { this.targets.delete(target); }
  disconnect(): void { this.targets.clear(); }
  takeRecords(): IntersectionObserverEntry[] { return []; }
  revealAll(): void {
    this.callback([...this.targets].map((target) => ({ isIntersecting: true, target }) as IntersectionObserverEntry), this);
  }
}

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetNearObserverForTests();
  resetStore();
  document.body.replaceChildren();
  ManualNearObserver.instance = null;
});

describe("standalone macro near-viewport deferral (GH #408)", () => {
  it("does not resolve an off-screen embed until its host enters the near band", async () => {
    vi.stubGlobal("IntersectionObserver", ManualNearObserver);
    const resolve = vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{
      page: "Source",
      kind: "page",
      blocks: [{ id: "target", raw: "resolved target", collapsed: false, children: [] }],
    }]);
    loadSingle({
      kind: "page",
      name: "Host",
      title: "Host",
      pre_block: null,
      format: "md",
      blocks: [{ id: "host", raw: "{{embed ((target))}}", collapsed: false, children: [] }],
    });

    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <Block id="host" />, host);
    try {
      expect(host.querySelector(".ast-deferred")?.textContent).toContain("{{embed");
      expect(host.querySelector(".embed-block")).toBeNull();
      expect(resolve).not.toHaveBeenCalled();

      ManualNearObserver.instance!.revealAll();
      await vi.waitFor(() => expect(host.querySelector(".embed-block")).not.toBeNull());
      await vi.waitFor(() => expect(resolve).toHaveBeenCalledWith(["target"]));
    } finally {
      dispose();
    }
  });
});
