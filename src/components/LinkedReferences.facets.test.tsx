import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { BacklinkFilterContext, BlockDto, RefGroup } from "../types";
import { LinkedReferences } from "./LinkedReferences";
import { resetReferenceSectionState } from "../referenceSectionState";

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

// GH #173 follow-up. The reporter asked that typing into the reference-text
// field also narrow the list he picks facets FROM ("an additional primary
// filter before making a selection"). The reference list itself already
// narrowed; the facet chips were computed over the UNFILTERED groups, so they
// and their counts never moved while he typed. OG's equivalent field
// ("Search in linked pages") narrows exactly that list.
describe("Linked References facet chips follow the text query (GH #173)", () => {
  const twoFacetedRoots = (): RefGroup[] => [
    {
      page: "Journal",
      kind: "journal",
      blocks: [block("alpha-root", "Planning [[My Project]]"), block("beta-root", "Other [[My Project]]")],
    },
  ];
  const facetedContext = (): BacklinkFilterContext => ({
    entries: [
      { page: "Journal", kind: "journal", block_id: "alpha-root", text: "Planning the needle", facets: ["Alpha"] },
      { page: "Journal", kind: "journal", block_id: "beta-root", text: "Other unrelated", facets: ["Beta"] },
    ],
  });

  const openFilter = async (root: HTMLElement) => {
    root.querySelector<HTMLButtonElement>('button[aria-label="Filter linked references"]')!.click();
    await tick();
    await tick();
  };
  const type = async (root: HTMLElement, query: string) => {
    const input = root.querySelector<HTMLInputElement>(".reference-filter-search")!;
    input.value = query;
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await wait(150);
  };
  const chipNames = (root: HTMLElement) =>
    [...root.querySelectorAll(".ref-filter-chip")].map((chip) => chip.textContent!.trim());
  const visibleIds = (root: HTMLElement) =>
    [...root.querySelectorAll<HTMLElement>(".test-ref-group")]
      .map((el) => el.textContent)
      .filter(Boolean)
      .join("|");

  it("drops facets whose only backlinks no longer match the typed text", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue(twoFacetedRoots());
    vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue(facetedContext());
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="My Project" />, root);
    await tick();
    await tick();
    await openFilter(root);

    expect(chipNames(root)).toEqual(["Alpha 1", "Beta 1"]);

    await type(root, "needle");
    expect(visibleIds(root)).toBe("alpha-root");
    expect(chipNames(root)).toEqual(["Alpha 1"]);

    await type(root, "");
    expect(chipNames(root)).toEqual(["Alpha 1", "Beta 1"]);
    dispose();
  });

  // NOTE: this one PASSES before the fix (chips were computed over every group,
  // so an active chip was trivially present). It is a guard against the
  // regression the fix could introduce, not a repro of the reported defect.
  it("keeps an already-selected facet visible so it can still be cleared", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue(twoFacetedRoots());
    vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue(facetedContext());
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="My Project" />, root);
    await tick();
    await tick();
    await openFilter(root);

    const beta = [...root.querySelectorAll<HTMLButtonElement>(".ref-filter-chip")]
      .find((chip) => chip.textContent!.includes("Beta"))!;
    beta.click();
    await tick();

    await type(root, "needle");
    // Beta no longer matches the text, but it is an ACTIVE filter: hiding its
    // chip would strand the user with an unclearable filter and zero results.
    const names = chipNames(root);
    expect(names.some((name) => name.startsWith("Beta"))).toBe(true);
    expect(names.some((name) => name.startsWith("Alpha"))).toBe(true);
    expect(
      root.querySelector<HTMLButtonElement>(".ref-filter-chip.f-in")?.textContent
    ).toContain("Beta");
    dispose();
  });
});

// GH #173 follow-up, second half. While the on-demand descendant index is in
// flight the component deliberately shows every group: the fallback corpus is a
// SUBSET of the native one, so dropping a root on a fallback miss could hide a
// real match. That is the right call — but the summary still read "N of N
// references", asserting a finished filter over an unfiltered list.
describe("Linked References filter summary is honest while indexing (GH #173)", () => {
  it("says filtering is pending instead of reporting a filtered count", async () => {
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([
      {
        page: "Journal",
        kind: "journal",
        blocks: [block("one", "A [[My Project]]"), block("two", "B [[My Project]]")],
      },
    ]);
    // Never resolves: the resource stays in its loading state.
    vi.spyOn(backend(), "getBacklinkFilterContext").mockReturnValue(
      new Promise<BacklinkFilterContext>(() => {})
    );
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <LinkedReferences name="My Project" />, root);
    await tick();
    await tick();
    root.querySelector<HTMLButtonElement>('button[aria-label="Filter linked references"]')!.click();
    await tick();

    const input = root.querySelector<HTMLInputElement>(".reference-filter-search")!;
    input.value = "needle";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await wait(150);

    const summary = root.querySelector(".reference-filter-summary")!.textContent!;
    expect(summary).toContain("Indexing");
    expect(summary).not.toContain("2 of 2");
    dispose();
  });
});
