import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { backend } from "../backend";
import type { BacklinkFilterContext, BlockDto, RefGroup } from "../types";
import { LinkedReferences } from "./LinkedReferences";
import { resetReferenceSectionState } from "../referenceSectionState";
import { setGraphMeta } from "../graphSession";
import * as searchPolicy from "../editor/searchFold";

vi.mock("./LiveRefGroup", () => ({
  LiveRefGroup: (props: { blocks: BlockDto[]; showBreadcrumb?: boolean }) => (
    <div class="test-ref-group" data-show-breadcrumb={props.showBreadcrumb ? "true" : "false"}>
      {props.blocks.map((block) => block.id).join(",")}
    </div>
  ),
}));

const block = (id: string, raw: string, marker?: string, children: BlockDto[] = []): BlockDto => ({
  id,
  raw,
  marker,
  collapsed: false,
  children,
});

function tick(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

afterEach(() => {
  document.body.innerHTML = "";
  localStorage.clear();
  resetReferenceSectionState();
  vi.restoreAllMocks();
});

describe("Linked References filters", () => {
  it("offers batch export for visible linked references", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([
      { page: "Source", kind: "page", blocks: [block("a", "[[Target]]")] },
    ]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <LinkedReferences name="Target" />, root);
    try {
      await tick(); await tick();
      expect(root.querySelector('[aria-label="Copy / export linked references"]')).not.toBeNull();
    } finally { dispose(); }
  });
  it("keeps an expanded large section open after its component remounts", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([{
      page: "Source", kind: "page",
      blocks: Array.from({ length: 100 }, (_, index) => block(`b${index}`, `[[Target]] ${index}`)),
    }]);
    const root = document.createElement("div");
    document.body.append(root);
    let dispose = render(() => <LinkedReferences name="Target" />, root);
    try {
      await tick(); await tick();
      expect(root.querySelector(".test-ref-group")).toBeNull();
      root.querySelector<HTMLElement>(".references-header")!.click();
      expect(root.querySelector(".test-ref-group")).not.toBeNull();
      dispose();
      dispose = render(() => <LinkedReferences name="Target" />, root);
      await tick(); await tick();
      expect(root.querySelector(".test-ref-group")).not.toBeNull();
    } finally { dispose(); }
  });
  it("unions include chips and narrows available chips to text matches", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([{
      page: "Source", kind: "page", blocks: [
        block("a", "apple [[Target]]"), block("b", "banana [[Target]]"),
      ],
    }]);
    vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue({ entries: [
      { page: "Source", kind: "page", block_id: "a", text: "apple", facets: ["red"] },
      { page: "Source", kind: "page", block_id: "b", text: "banana", facets: ["yellow"] },
    ] });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="Target" />, root);
    try {
      await tick(); await tick();
      root.querySelector<HTMLButtonElement>('[aria-label="Filter linked references"]')!.click();
      await tick(); await tick();
      const chip = (name: string) => [...root.querySelectorAll<HTMLButtonElement>(".ref-filter-chip")]
        .find((el) => el.textContent?.includes(name));
      chip("red")!.click(); chip("yellow")!.click();
      expect(root.querySelector(".references-count")?.textContent).toBe("2");
      const input = root.querySelector<HTMLInputElement>(".reference-filter-search")!;
      input.value = "apple";
      input.dispatchEvent(new Event("input", { bubbles: true }));
      await wait(150);
      expect(root.querySelector(".references-count")?.textContent).toBe("1");
      expect(chip("red")?.textContent).toContain("1");
      expect(chip("yellow")?.textContent).toContain("0");
    } finally { dispose(); }
  });
  it("folds NFC/NFD spellings of one tag into one filter chip (DUP-2)", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([{
      page: "Source", kind: "page", blocks: [block("a", "one [[Target]]"), block("b", "two [[Target]]")],
    }]);
    vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue({ entries: [
      { page: "Source", kind: "page", block_id: "a", text: "one", facets: ["Caf\u00e9"] },
      { page: "Source", kind: "page", block_id: "b", text: "two", facets: ["Cafe\u0301"] },
    ] });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="Target" />, root);
    try {
      await tick(); await tick();
      root.querySelector<HTMLButtonElement>('[aria-label="Filter linked references"]')!.click();
      await tick(); await tick();
      const chips = [...root.querySelectorAll<HTMLButtonElement>(".ref-filter-chip")]
        .filter((el) => el.textContent?.normalize("NFC").includes("Caf\u00e9"));
      expect(chips).toHaveLength(1);
      expect(chips[0].textContent).toContain("2");
    } finally { dispose(); }
  });
  it("ignores an old page's failed read after the reference target changes", async () => {
    let rejectOld!: (error: Error) => void;
    vi.spyOn(backend(), "getBacklinks")
      .mockImplementationOnce(() => new Promise((_, reject) => { rejectOld = reject; }))
      .mockResolvedValueOnce([]);
    const [name, setName] = createSignal("Old");
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name={name()} />, root);
    try {
      await tick();
      setName("New");
      await tick();
      rejectOld(new Error("old read failed"));
      await tick();
      expect(root.querySelector('[role="alert"]'), "I-20: a retired reference read cannot report an error on the new target").toBeNull();
    } finally {
      dispose();
    }
  });

  it("stays unmounted while loading and defaults a threshold-sized result to an unmounted body", async () => {
    let resolve!: (groups: RefGroup[]) => void;
    vi.spyOn(backend(), "getBacklinks").mockImplementation(
      () => new Promise<RefGroup[]>((done) => { resolve = done; })
    );
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="Target" />, root);

    await tick();
    expect(root.querySelector(".linked-references")).toBeNull();

    resolve([{
      page: "Source",
      kind: "page",
      blocks: Array.from({ length: 100 }, (_, index) => block(`b${index}`, `[[Target]] ${index}`)),
    }]);
    await tick();
    await tick();
    expect(root.querySelector(".references-count")?.textContent).toBe("100");
    expect(root.querySelector(".test-ref-group")).toBeNull();

    root.querySelector<HTMLElement>(".references-header")!.click();
    expect(root.querySelector(".test-ref-group")).not.toBeNull();
    dispose();
  });

  it("renders a bounded bridge error instead of an empty panel", async () => {
    vi.spyOn(backend(), "getBacklinks").mockRejectedValue(new Error("result-too-large"));
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="Target" />, root);

    await tick();
    await tick();
    expect(root.querySelector<HTMLElement>('[role="alert"]')?.textContent).toContain(
      "bounded result limit was exceeded"
    );
    dispose();
  });

  it("does not mislabel an ordinary backend failure as a bounded bridge error", async () => {
    vi.spyOn(backend(), "getBacklinks").mockRejectedValue(new Error("result-too-large: prose from another failure"));
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="Target" />, root);

    await tick();
    await tick();
    const message = root.querySelector<HTMLElement>('[role="alert"]')?.textContent ?? "";
    expect(message).toContain("Couldn’t load references");
    expect(message).not.toContain("bounded result limit");
    dispose();
  });

  it("requests ancestor context for every linked-reference hit", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([
      { page: "Journal", kind: "journal", blocks: [block("nested", "Nested [[Target]]")] },
    ]);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="Target" />, root);

    await tick();
    await tick();
    expect(root.querySelector(".test-ref-group")?.getAttribute("data-show-breadcrumb")).toBe("true");

    dispose();
  });

  it("normalizes each native search corpus once instead of once per search evaluation", async () => {
    const groups: RefGroup[] = [
      {
        page: "Journal",
        kind: "journal",
        blocks: [block("root", "Planning [[My Project]]")],
      },
    ];
    const indexedText = "UNIQUE INDEXED SEARCH CORPUS";
    const originalSearchFold = searchPolicy.searchFold;
    let corpusNormalizations = 0;
    // The casing now runs in Rust; count the same normalization operation at
    // its public door, retaining the once-per-corpus cost assertion.
    vi.spyOn(searchPolicy, "searchFold").mockImplementation((value, removeAccents) => {
      if (value === indexedText) corpusNormalizations += 1;
      return originalSearchFold(value, removeAccents);
    });
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
    vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue({
      entries: [
        { page: "Journal", kind: "journal", block_id: "root", text: indexedText, facets: [] },
      ],
    });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="My Project" />, root);

    await tick();
    await tick();
    root.querySelector<HTMLButtonElement>('button[aria-label="Filter linked references"]')!.click();
    await tick();
    await tick();
    expect(corpusNormalizations).toBe(1);

    const input = root.querySelector<HTMLInputElement>(".reference-filter-search")!;
    for (const query of ["unique", "indexed", "search corpus"]) {
      input.value = query;
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await wait(150);
      expect(root.querySelector(".references-count")?.textContent).toBe("1");
    }
    expect(corpusNormalizations).toBe(1);

    dispose();
  });

  it("keeps a backlink root when ephemeral content search matches only a descendant (GH #173)", async () => {
    const groups: RefGroup[] = [
      {
        page: "Journal",
        kind: "journal",
        blocks: [
          block("matching-root", "Planning [[My Project]]"),
          block("other-root", "Another [[My Project]] reference"),
        ],
      },
    ];
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
    vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue({
      entries: [
        { page: "Journal", kind: "journal", block_id: "matching-root", text: "Planning\nA descendant carries the exact needle", facets: [] },
        { page: "Journal", kind: "journal", block_id: "other-root", text: "Another reference\nUnrelated descendant", facets: [] },
      ],
    });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="My Project" />, root);

    await tick();
    await tick();
    const filterButton = root.querySelector<HTMLButtonElement>(
      'button[aria-label="Filter linked references"]'
    );
    expect(filterButton).not.toBeNull();
    filterButton!.click();
    await tick();

    const input = root.querySelector<HTMLInputElement>('.reference-filter-search');
    expect(input).not.toBeNull();
    input!.value = '"exact needle"';
    input!.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await wait(150);

    expect(root.querySelector(".reference-filter-summary")?.textContent).toContain("1 of 2");
    expect(root.querySelector(".test-ref-group")?.textContent).toBe("matching-root");

    root.querySelector<HTMLButtonElement>(".reference-filter-clear")!.click();
    await tick();
    expect(root.querySelector(".reference-filter-summary")?.textContent).toContain("2 of 2");
    expect(root.querySelector(".test-ref-group")?.textContent).toBe("matching-root,other-root");

    dispose();
  });

  it("includes task markers and references from child blocks", async () => {
    const groups: RefGroup[] = [
      {
        page: "Jul 10th, 2026",
        kind: "journal",
        blocks: [
          block("planning", "Planning [[My Project]] #fun #pin", undefined, [
            block("nested-todo", "TODO maybe not to be detected", "TODO"),
          ]),
        ],
      },
      {
        page: "Jul 9th, 2026",
        kind: "journal",
        blocks: [
          block("sync", "Sync [[My Project]] #fun", undefined, [
            block("nested-pin", "very important note #pin"),
          ]),
          block("direct-todo", "TODO should be detected [[My Project]]", "TODO"),
        ],
      },
    ];
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
    const context: BacklinkFilterContext = {
      entries: [
        { page: "Jul 10th, 2026", kind: "journal", block_id: "planning", text: "Planning", facets: ["fun", "pin", "TODO"] },
        { page: "Jul 9th, 2026", kind: "journal", block_id: "sync", text: "Sync", facets: ["fun", "pin"] },
        { page: "Jul 9th, 2026", kind: "journal", block_id: "direct-todo", text: "TODO should be detected", facets: ["TODO"] },
      ],
    };
    vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue(context);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="My Project" />, root);

    await tick();
    await tick();
    root.querySelector<HTMLButtonElement>('button[aria-label="Filter linked references"]')!.click();
    await tick();

    const chips = [...root.querySelectorAll<HTMLButtonElement>(".ref-filter-chip")].map((el) =>
      el.textContent?.replace(/\s+/g, " ").trim()
    );
    expect(chips).toContain("TODO 2");
    expect(chips).toContain("pin 2");

    dispose();
  });

  it("filters a backlink root when its descendant has the selected marker", async () => {
    const groups: RefGroup[] = [
      {
        page: "Journal",
        kind: "journal",
        blocks: [
          block("with-task", "Planning [[My Project]] #work", undefined, [
            block("task", "TODO nested task", "TODO"),
          ]),
          block("without-task", "Notes [[My Project]] #notes"),
        ],
      },
    ];
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
    vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue({
      entries: [
        { page: "Journal", kind: "journal", block_id: "with-task", text: "Planning\nTODO nested task", facets: ["work", "TODO"] },
        { page: "Journal", kind: "journal", block_id: "without-task", text: "Notes", facets: ["notes"] },
      ],
    });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="My Project" />, root);

    await tick();
    await tick();
    root.querySelector<HTMLButtonElement>('button[aria-label="Filter linked references"]')!.click();
    await tick();
    const todo = [...root.querySelectorAll<HTMLButtonElement>(".ref-filter-chip")].find((el) =>
      el.textContent?.includes("TODO")
    );
    expect(todo).toBeDefined();
    todo!.click();

    expect(root.querySelector(".references-count")?.textContent).toBe("1");
    expect(root.querySelector(".test-ref-group")?.textContent).toBe("with-task");

    dispose();
  });
});

// GH #479 (master 97b26be8a). A page opens its Linked References collapsed
// once the TOTAL backlink count reaches `:ref/linked-references-collapsed-threshold`
// (OG `(>= total threshold)`, default 100). Zero means "always collapsed".
describe("Linked References honor :ref/linked-references-collapsed-threshold (GH #479)", () => {
  const backlinks = (count: number): RefGroup[] => [{
    page: "Source",
    kind: "page",
    blocks: Array.from({ length: count }, (_, index) => block(`b${index}`, `[[Target]] ${index}`)),
  }];

  async function mountWithThreshold(count: number, threshold?: number) {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue(backlinks(count));
    setGraphMeta(
      threshold === undefined
        ? ({ root: "/graphs/A" } as never)
        : ({ root: "/graphs/A", linked_references_collapsed_threshold: threshold } as never),
    );
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="Target" />, root);
    await tick(); await tick();
    return { root, dispose: () => { dispose(); setGraphMeta(null); } };
  }

  it("starts collapsed at a threshold of 0, however few backlinks there are", async () => {
    const { root, dispose } = await mountWithThreshold(3, 0);
    expect(root.querySelector(".test-ref-group")).toBeNull();
    dispose();
  });

  it("starts expanded below a configured threshold and collapsed at it", async () => {
    const below = await mountWithThreshold(4, 5);
    expect(below.root.querySelector(".test-ref-group")).not.toBeNull();
    below.dispose();

    const at = await mountWithThreshold(5, 5);
    expect(at.root.querySelector(".test-ref-group")).toBeNull();
    at.dispose();
  });

  it("falls back to OG's 100 when the graph does not set the key", async () => {
    const under = await mountWithThreshold(99);
    expect(under.root.querySelector(".test-ref-group")).not.toBeNull();
    under.dispose();

    const over = await mountWithThreshold(100);
    expect(over.root.querySelector(".test-ref-group")).toBeNull();
    over.dispose();
  });
});

describe("Linked References page header long-press (GH #207)", () => {
  it("opens the page context menu on a still touch hold and swallows the release click", async () => {
    const { contextMenu, closeContextMenu } = await import("../ui");
    const { route, openPage } = await import("../router");
    const { LONG_PRESS_DELAY } = await import("../render/longPress");
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([
      { page: "Backlink Owner", kind: "page", blocks: [block("a", "[[Target]]")] },
    ]);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <LinkedReferences name="Target" />, root);
    try {
      await tick(); await tick();
      const header = root.querySelector<HTMLButtonElement>(".reference-page")!;
      expect(header.textContent).toBe("Backlink Owner");
      openPage("Elsewhere", "page");
      vi.useFakeTimers();
      const touch = (type: string) => new PointerEvent(type, {
        bubbles: true, cancelable: true, pointerType: "touch", isPrimary: true, pointerId: 7, clientX: 20, clientY: 30,
      });
      header.dispatchEvent(touch("pointerdown"));
      vi.advanceTimersByTime(LONG_PRESS_DELAY);
      expect(contextMenu()).toMatchObject({ kind: "page", name: "Backlink Owner" });
      header.dispatchEvent(touch("pointerup"));
      header.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
      const current = route();
      expect(current.kind === "page" ? current.name : current.kind).toBe("Elsewhere");
    } finally {
      vi.useRealTimers();
      closeContextMenu();
      dispose();
    }
  });
});
