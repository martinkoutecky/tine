import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { BacklinkFilterContext, BlockDto, RefGroup } from "../types";
import { LinkedReferences } from "./LinkedReferences";

vi.mock("./LiveRefGroup", () => ({
  LiveRefGroup: (props: { blocks: BlockDto[] }) => (
    <div class="test-ref-group">{props.blocks.map((block) => block.id).join(",")}</div>
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
  vi.restoreAllMocks();
});

describe("Linked References filters", () => {
  it("normalizes each native search corpus once instead of once per search evaluation", async () => {
    const groups: RefGroup[] = [
      {
        page: "Journal",
        kind: "journal",
        blocks: [block("root", "Planning [[My Project]]")],
      },
    ];
    const indexedText = "UNIQUE INDEXED SEARCH CORPUS";
    const originalToLowerCase = String.prototype.toLowerCase;
    let corpusNormalizations = 0;
    vi.spyOn(String.prototype, "toLowerCase").mockImplementation(function (this: string) {
      if (String(this) === indexedText) corpusNormalizations += 1;
      return originalToLowerCase.call(this);
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
