import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { loadSingle, resetStore } from "../store";
import type { BlockDto, PageDto, RefGroup } from "../types";
import { Block } from "./Block";
import { LinkDepthContext } from "./linkDepth";

let liveGroupBudget = Number.POSITIVE_INFINITY;
let observedLiveGroups = 0;

class BudgetIntersectionObserver implements IntersectionObserver {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [0];

  constructor(private readonly callback: IntersectionObserverCallback) {}

  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] { return []; }
  unobserve(): void {}

  observe(target: Element): void {
    if (target.classList.contains("live-ref-group")) {
      if (observedLiveGroups >= liveGroupBudget) return;
      observedLiveGroups += 1;
    }
    this.callback([{ isIntersecting: true, target } as IntersectionObserverEntry], this);
  }
}

beforeAll(async () => {
  await initParser();
});

beforeEach(() => {
  liveGroupBudget = Number.POSITIVE_INFINITY;
  observedLiveGroups = 0;
  vi.stubGlobal("IntersectionObserver", BudgetIntersectionObserver);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetStore();
  document.body.innerHTML = "";
});

function mountBlock(id: string, initialLinkDepth = 0) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => (
    <LinkDepthContext.Provider value={initialLinkDepth}>
      <Block id={id} />
    </LinkDepthContext.Provider>
  ), root);
  return { root, dispose };
}

function page(name: string, blocks: BlockDto[]): PageDto {
  return { name, title: name, kind: "page", pre_block: null, blocks };
}

function mockBlockEmbeds(sourcePage: PageDto, roots: BlockDto[]): void {
  const groups = new Map<string, RefGroup>(roots.map((block) => [block.id, {
    page: sourcePage.name,
    kind: sourcePage.kind,
    blocks: [{ ...block, children: [] }],
  }]));
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) =>
    ids.map((id) => groups.get(id) ?? null)
  );
}

function embedChain(prefix: string, length: number): { page: PageDto; roots: BlockDto[]; hostId: string } {
  const roots: BlockDto[] = [];
  const all: BlockDto[] = [];
  for (let index = 0; index < length; index += 1) {
    const child: BlockDto | null = index + 1 < length ? {
      id: `${prefix}-embed-${index}`,
      raw: `{{embed ((${prefix}-target-${index + 1}))}}`,
      collapsed: false,
      children: [],
    } : null;
    const root: BlockDto = {
      id: `${prefix}-target-${index}`,
      raw: `${prefix} level ${index}`,
      collapsed: false,
      children: child ? [child] : [],
    };
    roots.push(root);
    all.push(root);
  }
  const hostId = `${prefix}-host`;
  all.push({
    id: hostId,
    raw: `{{embed ((${prefix}-target-0))}}`,
    collapsed: false,
    children: [],
  });
  return { page: page(`${prefix} page`, all), roots, hostId };
}

describe("shared embed/ref render depth (GH #206)", () => {
  it("bounds a block embedded beneath itself and shows the OG depth notice", async () => {
    const targetId = "recursive-target-206";
    const embeddedHost: BlockDto = {
      id: "recursive-host-206",
      raw: `{{embed ((${targetId}))}}`,
      collapsed: false,
      children: [],
    };
    const target: BlockDto = {
      id: targetId,
      raw: "Recursive embed target",
      collapsed: false,
      children: [embeddedHost],
    };
    const sourcePage = page("Recursive Embed", [target]);
    mockBlockEmbeds(sourcePage, [target]);
    loadSingle(sourcePage);
    // This safety-only observer budget keeps the pre-fix proof finite. The product
    // guard must stop earlier and render its notice without relying on the budget.
    liveGroupBudget = 8;

    // Begin close to the cap so the exact recursive topology reaches the guard
    // deterministically even in jsdom, which throttles repeated identical trees.
    const { root, dispose } = mountBlock(targetId, 4);
    try {
      await vi.waitFor(() => expect(root.querySelectorAll(`[data-block-id="${targetId}"]`).length).toBeGreaterThan(1));
      await vi.waitFor(() => expect(root.textContent).toContain("Embed depth is too deep"));
      expect(root.querySelectorAll(`[data-block-id="${targetId}"]`).length).toBeLessThanOrEqual(7);
      expect(observedLiveGroups).toBeLessThan(liveGroupBudget);
    } finally {
      dispose();
    }
  });

  it("does not mount a page inside its own block", async () => {
    const hostId = "page-self-host-206";
    const selfPage = page("Self Embed Page", [{
      id: hostId,
      raw: "{{embed [[Self Embed Page]]}}",
      collapsed: false,
      children: [],
    }]);
    loadSingle(selfPage);
    liveGroupBudget = 2;
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(selfPage);

    const { root, dispose } = mountBlock(hostId);
    try {
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(root.querySelectorAll(".live-ref-group")).toHaveLength(0);
      expect(getPage).not.toHaveBeenCalled();
    } finally {
      dispose();
    }
  });

  it("stops a finite embed chain deeper than five", async () => {
    const fixture = embedChain("deep-206", 8);
    mockBlockEmbeds(fixture.page, fixture.roots);
    loadSingle(fixture.page);

    const { root, dispose } = mountBlock(fixture.hostId);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("Embed depth is too deep"));
      expect(root.textContent).toContain("deep-206 level 5");
      expect(root.textContent).not.toContain("deep-206 level 6");
    } finally {
      dispose();
    }
  });

  it("keeps ordinary one-level and three-level embeds fully rendered", async () => {
    for (const length of [1, 3]) {
      const fixture = embedChain(`legit-${length}-206`, length);
      mockBlockEmbeds(fixture.page, fixture.roots);
      loadSingle(fixture.page);
      const { root, dispose } = mountBlock(fixture.hostId);
      try {
        await vi.waitFor(() => expect(root.textContent).toContain(`legit-${length}-206 level ${length - 1}`));
        expect(root.textContent).not.toContain("Embed depth is too deep");
      } finally {
        dispose();
      }
      resetStore();
    }
  });

  it("bounds a query that returns its own block through the same live-render context", async () => {
    const queryId = "recursive-query-206";
    const queryBlock: BlockDto = {
      id: queryId,
      raw: "{{query (task TODO)}}",
      collapsed: false,
      children: [],
    };
    const sourcePage = page("Recursive Query", [queryBlock]);
    loadSingle(sourcePage);
    vi.spyOn(backend(), "runQuery").mockResolvedValue([{
      page: sourcePage.name,
      kind: sourcePage.kind,
      blocks: [{ ...queryBlock }],
    }]);
    liveGroupBudget = 8;

    const { root, dispose } = mountBlock(queryId, 4);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("Embed depth is too deep"));
      expect(root.querySelectorAll(`[data-block-id="${queryId}"]`).length).toBeLessThanOrEqual(7);
      expect(observedLiveGroups).toBeLessThan(liveGroupBudget);
    } finally {
      dispose();
    }
  });
});
