import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ensurePageLoaded, pageByName, resetStore, setRaw } from "../document";
import { resetNearObserverForTests } from "../lazyObserve";
import { resetReferenceSectionState } from "../referenceSectionState";
import type { BlockDto, PageRead, RefGroup } from "../types";
import { LinkedReferences } from "./LinkedReferences";
import { queueReferenceRead } from "../referenceAdmission";

afterEach(() => {
  resetNearObserverForTests();
  resetStore();
  resetReferenceSectionState();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
  document.body.replaceChildren();
});

const block = (id: string): BlockDto => ({ id, raw: `[[Hub]] ${id}`, collapsed: false, children: [] });
const page = (name: string, blocks: BlockDto[]): PageRead => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", format: "md", pre_block: "", blocks,
});

it.each([false, true])("loading one source page leaves every other Linked References group's DOM untouched (routed=%s)", async (routed) => {
  // This tests DOM isolation, not wall-clock admission speed. Group shells and
  // reads yield through frames/tasks, which can span the wait deadline on a busy
  // gate. Drive those turns along with the source completions controlled below.
  vi.useFakeTimers();
  const groups: RefGroup[] = Array.from({ length: 6 }, (_, n) => ({
    page: `Source ${n}`, kind: "page", blocks: [
      { ...block(`row-${n}`), tags: ["Hub"], breadcrumb: n === 2 ? ["Parent"] : [] },
    ],
  }));
  ensurePageLoaded(page("Hub", [block("hub")]));
  // Keep source reads pending: the test controls each load separately.
  const pending = new Map<string, (dto: PageRead) => void>();
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
  vi.spyOn(backend(), "getPage").mockImplementation(name => new Promise(resolve => pending.set(name, resolve)));
  vi.stubGlobal("IntersectionObserver", class {
    constructor(private callback: IntersectionObserverCallback) {}
    observe(target: Element) {
      queueMicrotask(() => this.callback([{ target, isIntersecting: true } as IntersectionObserverEntry], this as unknown as IntersectionObserver));
    }
    unobserve() {}
    disconnect() {}
  });
  const root = document.createElement("div");
  document.body.append(root);
  // Page.tsx passes its name through pagesToRender(), whose pageByName lookup
  // also observes unrelated source loads. The visible name still stays Hub.
  const dispose = render(() => <LinkedReferences name={routed ? pageByName("Hub")!.name : "Hub"} />, root);
  try {
    await vi.runAllTimersAsync();
    expect(pending.size).toBe(2);
    expect(root.querySelectorAll(".ref-block")).toHaveLength(groups.length);
    const elements = [...root.querySelectorAll<HTMLElement>(".reference-group")];
    const lastDisclosure = elements.at(-1)!.querySelector<HTMLButtonElement>(".reference-group-disclosure")!;
    lastDisclosure.click();
    expect(lastDisclosure.getAttribute("aria-expanded")).toBe("false");
    const bodies = [...root.querySelectorAll(".reference-blocks")];
    // This merge-base predates the GH #849 breadcrumb/filter/header follow-up.
    // Keep filters/headers absent; load top-level rows, then a nested row last:
    // the real components exercise section invalidation without requiring that fix.
    for (const group of groups.slice(0, 3)) {
      expect(pending.has(group.page)).toBe(true);
      const touched = new Set<string>();
      const observer = new MutationObserver(records => {
        for (const record of records) {
          const element = record.target instanceof Element ? record.target : record.target.parentElement;
          const owner = element?.closest(".reference-group");
          if (owner) touched.add(owner.querySelector(".reference-page")!.textContent!);
        }
      });
      observer.observe(root, { subtree: true, attributes: true, childList: true, characterData: true });
      try {
        const rows = group.blocks;
        const blocks = group.page === "Source 2"
          ? [{ ...block(`parent-${group.page}`), raw: "Parent", children: rows }]
          : rows;
        pending.get(group.page)!(page(group.page, blocks));
        await vi.runAllTimersAsync();
        expect(root.querySelector(`.ls-block:not(.ref-block)[data-block-id="${group.blocks[0].id}"]`)).not.toBeNull();
        expect([...touched]).toEqual([group.page]);
        const currentGroups = [...root.querySelectorAll(".reference-group")];
        const currentBodies = [...root.querySelectorAll(".reference-blocks")];
        expect(currentGroups).toHaveLength(elements.length);
        expect(currentBodies).toHaveLength(bodies.length);
        currentGroups.forEach((element, index) => expect(element).toBe(elements[index]));
        currentBodies.forEach((element, index) => expect(element).toBe(bodies[index]));
        expect(lastDisclosure.getAttribute("aria-expanded")).toBe("false");
      } finally { observer.disconnect(); }
    }
    // Value stability must still let a real source edit update its breadcrumb.
    expect(elements[2].querySelector(".ref-breadcrumb")?.textContent).toContain("Parent");
    setRaw("parent-Source 2", "Changed parent");
    await vi.runAllTimersAsync();
    expect(elements[2].querySelector(".ref-breadcrumb")?.textContent).toContain("Changed parent");
  } finally {
    dispose();
    for (const [name, resolve] of pending) resolve(page(name, []));
    const drained = queueReferenceRead(async () => true, () => true, () => 0);
    await vi.runAllTimersAsync();
    await drained;
  }
});
