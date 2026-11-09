import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ensurePageLoaded, resetStore, setRaw } from "../document";
import { resetNearObserverForTests } from "../lazyObserve";
import { resetReferenceSectionState } from "../referenceSectionState";
import type { BlockDto, PageRead, RefGroup } from "../types";
import { LinkedReferences } from "./LinkedReferences";
import { queueReferenceCommit, queueReferenceRead } from "../referenceAdmission";

afterEach(() => {
  resetNearObserverForTests();
  resetStore();
  resetReferenceSectionState();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
  document.body.replaceChildren();
});

const block = (id: string): BlockDto => ({ id, raw: `[[Hub]] ${id}`, collapsed: false, children: [] });
const page = (name: string, blocks: BlockDto[]): PageRead => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", format: "md", pre_block: "", blocks,
});

it.each([null, 'filters:: {"Hub" true}'])("loading one source page leaves every other Linked References group's DOM untouched (filters=%s)", async (filters) => {
  const groups: RefGroup[] = Array.from({ length: 6 }, (_, n) => ({
    page: `Source ${n}`, kind: "page", blocks: [{ ...block(`row-${n}`), tags: ["Hub"], breadcrumb: ["Parent"] }],
  }));
  ensurePageLoaded({ ...page("Hub", [block("hub")]), pre_block: filters });
  // Keep source reads pending: the test controls each load separately.
  const pending = new Map<string, (dto: PageRead) => void>();
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
  vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue({ entries: groups.map(group => ({
    page: group.page, kind: group.kind, block_id: group.blocks[0].id, text: group.blocks[0].raw, facets: ["Hub"],
  })) });
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
  const dispose = render(() => <LinkedReferences name="Hub" />, root);
  try {
    await queueReferenceCommit(() => undefined, () => true, () => Infinity);
    await vi.waitFor(() => expect(pending.size).toBe(2));
    await vi.waitFor(() => expect(root.querySelectorAll(".ref-block")).toHaveLength(groups.length));
    await new Promise(resolve => setTimeout(resolve, 0));
    const elements = [...root.querySelectorAll<HTMLElement>(".reference-group")];
    for (const group of groups.slice(0, 3)) {
      await vi.waitFor(() => expect(pending.has(group.page)).toBe(true));
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
        pending.get(group.page)!(page(group.page, [{ ...block(`parent-${group.page}`), raw: "Parent", children: group.blocks }]));
        await vi.waitFor(() => expect(root.querySelector(`.ls-block:not(.ref-block)[data-block-id="${group.blocks[0].id}"]`)).not.toBeNull());
        await new Promise(resolve => setTimeout(resolve, 0));
        expect([...touched]).toEqual([group.page]);
        expect([...root.querySelectorAll(".reference-group")]).toEqual(elements);
      } finally { observer.disconnect(); }
    }
    // Value stability must still let a real source edit update its breadcrumb.
    setRaw("parent-Source 0", "Changed parent");
    await vi.waitFor(() => expect(elements[0].querySelector(".ref-breadcrumb")?.textContent).toContain("Changed parent"));
  } finally {
    dispose();
    for (const [name, resolve] of pending) resolve(page(name, []));
    // Mock native reads must settle too: the shared two-read bound spans views.
    await queueReferenceRead(async () => true, () => true, () => 0);
  }
});
