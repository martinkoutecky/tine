import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ensurePageLoaded, pageByName, resetStore } from "../document";
import { resetNearObserverForTests } from "../lazyObserve";
import { initParser } from "../render/parse";
import type { PageDto, RefGroup } from "../types";
import { ReferenceSection } from "./LinkedReferences";

beforeAll(initParser);
afterEach(() => {
  resetNearObserverForTests();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  resetStore();
  document.body.innerHTML = "";
});

const name = (i: number) => `Source ${i}`;
const row = (i: number) => ({ id: `row-${i}`, uuid: `row-${i}`, raw: `Row ${i} about [[Hub]]`, children: [], collapsed: false });
const dto = (i: number): PageDto => ({ id: `pages/${name(i)}.md`, name: name(i), kind: "page", title: name(i), pre_block: null, blocks: [row(i)], format: "md" } as unknown as PageDto);
const settle = async (reads: () => number) => {
  // Quiet means no read through several timer turns, each of which lets a thrash take its next step.
  let last = -1;
  for (let quiet = 0; quiet < 8;) {
    await new Promise((resolve) => setTimeout(resolve, 15));
    const now = reads();
    quiet = now === last ? quiet + 1 : 0;
    last = now;
    if (now > 2000) break;
  }
  return last;
};

describe("GH #849: linked references over more pages than the working set holds", () => {
  const mount = async (count: number) => {
    const callbacks = new Set<(element: Element, near: boolean) => void>();
    const near = new Map<Element, boolean>();
    let isNear = (_: Element) => true;
    vi.stubGlobal("IntersectionObserver", class {
      observe(target: Element) {
        this.targets.add(target);
        near.set(target, isNear(target));
        queueMicrotask(() => { if (near.has(target)) this.deliver(target, isNear(target)); });
      }
      constructor(private callback: IntersectionObserverCallback) {
        callbacks.add((element, value) => { if (this.observed.has(element)) this.deliver(element, value); });
      }
      observed = new Set<Element>();
      // Disconnecting one observer forgets only its own targets, as in browsers:
      // block bodies use their own observer, released once every body has mounted.
      targets = new Set<Element>();
      deliver(target: Element, isIntersecting: boolean) {
        this.observed.add(target);
        this.callback([{ target, isIntersecting } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
      }
      unobserve(target: Element) { near.delete(target); this.observed.delete(target); this.targets.delete(target); }
      disconnect() { for (const target of this.targets) near.delete(target); this.targets.clear(); this.observed.clear(); }
    });
    const groups: RefGroup[] = Array.from({ length: count }, (_, i) => ({ page: name(i), kind: "page", blocks: [row(i)] } as unknown as RefGroup));
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
    const read = vi.spyOn(backend(), "getPage").mockImplementation(async (page: string) => dto(Number(page.split(" ")[1])) as never);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <ReferenceSection name="Hub" />, root);
    return { read, root, dispose, setNear(rule: (i: number) => boolean) {
      const index = (host: Element) => [...root.querySelectorAll(".live-ref-group")].indexOf(host);
      isNear = (host) => rule(index(host));
      for (const host of [...near.keys()]) for (const deliver of callbacks) deliver(host, isNear(host));
    } };
  };

  it("an idle page reads each source page once when all of them are near the view", async () => {
    const { read, root, dispose } = await mount(90);
    try {
      await expect.poll(() => root.querySelectorAll(".live-ref-group").length).toBe(90);
      const reads = await settle(() => read.mock.calls.length);
      expect(reads, "more source pages near the view than the working set holds must not evict each other").toBe(90);
      for (let i = 0; i < 90; i++) expect(pageByName(name(i))?.name).toBe(name(i));
    } finally { dispose(); }
  }, 30_000);

  it("a page evicted while away loads again when its group comes back, and only then", async () => {
    const { read, root, dispose, setNear } = await mount(90);
    try {
      await expect.poll(() => root.querySelectorAll(".live-ref-group").length).toBe(90);
      await settle(() => read.mock.calls.length);
      setNear((i) => i >= 80);
      // Nothing is released until a load needs room: moving away does no work.
      expect(await settle(() => read.mock.calls.length)).toBe(90);
      expect(ensurePageLoaded(dto(1000))).toBeNull();
      expect(pageByName(name(0)), "the oldest page away from the view makes room").toBeUndefined();
      expect(pageByName(name(80)), "a page near the view stays").toBeDefined();
      expect(await settle(() => read.mock.calls.length), "an evicted page away from the view is not read back").toBe(90);
      setNear((i) => i === 0 || i >= 80);
      await expect.poll(() => pageByName(name(0))?.name).toBe(name(0));
      expect(await settle(() => read.mock.calls.length), "coming back reads it once").toBe(91);
    } finally { dispose(); }
  }, 30_000);
});
