import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import type { RefGroup } from "../types";
import { resetStore } from "../document";
import { endEdit } from "../editorController";
import { bumpDataRev } from "../graphSession";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { LinkedReferences } from "./LinkedReferences";

// UI-GH746-REFERENCE-REFRESH-COST: after each save the reference sections read
// their answer again. That refresh must not rebuild the filter index nobody
// uses, flash "Loading", or remount rows whose content did not change.
const folds = vi.hoisted(() => ({ count: 0 }));
vi.mock("../editor/searchFold", async (original) => {
  const real = await original<typeof import("../editor/searchFold")>();
  return { searchFold: (value: string, removeAccents?: boolean) => { folds.count += 1; return real.searchFold(value, removeAccents); } };
});

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
  endEdit("blur");
  resetStore();
});

// A fresh answer of equal content, as every refresh delivers it.
function answer(groups = 40): RefGroup[] {
  return Array.from({ length: groups }, (_, g) => ({
    page: `Source ${g}`,
    kind: "page" as const,
    blocks: [{ id: `b${g}`, raw: `a Target mention ${g}`, collapsed: false, children: [
      { id: `c${g}`, raw: `child ${g} #Other`, collapsed: false, children: [], tags: ["Other"] },
    ] }],
    evidence: [{ block_id: `b${g}`, occurrences: [
      { matched_name: "Target", canonical: "Target", kind: "plain" as const, span: { start: 2, end: 8 }, rule: "plain" },
    ] }],
  }));
}

function mount(view: () => ReturnType<typeof UnlinkedReferences>) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(view, root) };
}

it("refreshes unlinked and linked references without building the unused filter index", async () => {
  const unlinked = vi.spyOn(backend(), "getUnlinkedRefs").mockImplementation(async () => answer());
  const linked = vi.spyOn(backend(), "getBacklinks").mockImplementation(async () => answer());
  const { root, dispose } = mount(() => <><LinkedReferences name="Target" /><UnlinkedReferences name="Target" /></>);
  try {
    await vi.waitFor(() => expect(root.querySelector(".linked-references")).not.toBeNull());
    folds.count = 0;
    for (let save = 0; save < 3; save++) {
      bumpDataRev();
      await tick();
      await tick();
    }
    expect(unlinked).toHaveBeenCalledTimes(4);
    expect(linked).toHaveBeenCalledTimes(4);
    expect(folds.count, "no search fold per reference root while no filter is in use").toBe(0);

    // The index still exists the moment the filter panel asks for it.
    vi.spyOn(backend(), "getBacklinkFilterContext").mockImplementation(() => new Promise(() => {}));
    root.querySelector<HTMLButtonElement>(".linked-references .reference-filter-toggle")!.click();
    await tick();
    expect(folds.count).toBeGreaterThan(0);
    expect(root.querySelector(".linked-references .ref-filter-chip")?.textContent).toContain("Other");
  } finally {
    dispose();
  }
});

it("keeps shown unlinked rows in place, without Loading, while a refresh with the same content runs", async () => {
  let finish: ((groups: RefGroup[]) => void) | undefined;
  vi.spyOn(backend(), "getUnlinkedRefs").mockImplementation(() => finish
    ? new Promise<RefGroup[]>((resolve) => { finish = resolve; })
    : Promise.resolve(answer(3)));
  const { root, dispose } = mount(() => <UnlinkedReferences name="Target" />);
  try {
    await vi.waitFor(() => expect(root.querySelector(".references-count")?.textContent).toBe("3"));
    root.querySelector<HTMLElement>(".references-header")!.click();
    await vi.waitFor(() => expect(root.querySelector('[data-reference-block="b1"]')).not.toBeNull());
    const row = root.querySelector('[data-reference-block="b1"]');

    finish = () => {};
    bumpDataRev();
    await tick();
    expect(root.querySelector(".unlinked-references")?.textContent, "a refresh keeps the answer on screen").not.toContain("Loading");
    finish(answer(3));
    await tick();
    await tick();
    expect(root.querySelector('[data-reference-block="b1"]'), "an unchanged row is not mounted again").toBe(row);
  } finally {
    dispose();
  }
});
