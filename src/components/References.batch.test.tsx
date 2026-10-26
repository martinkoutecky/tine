import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { LinkedReferences } from "./LinkedReferences";
import { UnlinkedReferences } from "./UnlinkedReferences";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetReferenceSectionState } from "../referenceSectionState";
import { noteSurfaceFocused, startEditing, endEdit } from "../editorController";
import { bumpDataRev } from "../graphSession";

beforeAll(initParser);
afterEach(() => { endEdit("blur"); noteSurfaceFocused("main"); vi.restoreAllMocks(); localStorage.clear(); resetReferenceSectionState(); document.body.replaceChildren(); });

it("journal reference-result editors retain their answer while feed-source editors may refresh", async () => {
  const read = vi.spyOn(backend(), "getBacklinks").mockResolvedValue([{ page: "Source", kind: "page", blocks: [{ id: "retained", raw: "reference", children: [], collapsed: false }] }]);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <LinkedReferences name="Target" journalFeed />, host);
  try {
    await vi.waitFor(() => expect(host.querySelector(".references-count")?.textContent).toBe("1"));
    noteSurfaceFocused("ref:test"); startEditing("retained");
    read.mockResolvedValue([]); bumpDataRev();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(host.querySelector(".references-count")?.textContent).toBe("1");
    endEdit("blur");
    await vi.waitFor(() => expect(host.querySelector(".linked-references")).toBeNull());
  } finally { dispose(); }
});

it("GH #398: copy precedes filter; unlinked filtering has independent text and facets", async () => {
  const groups = [{ page: "Source", kind: "page" as const, blocks: [
    { id: "apple", raw: "Target apple", children: [], collapsed: false },
    { id: "banana", raw: "Target banana", children: [], collapsed: false },
  ] }];
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue(groups);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue(groups);
  vi.spyOn(backend(), "getBacklinkFilterContext").mockResolvedValue({ entries: [
    { page: "Source", kind: "page", block_id: "apple", text: "Target apple", facets: ["red"] },
    { page: "Source", kind: "page", block_id: "banana", text: "Target banana", facets: ["yellow"] },
  ] });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <><LinkedReferences name="Target" /><UnlinkedReferences name="Target" /></>, host);
  try {
    await vi.waitFor(() => expect(host.querySelectorAll(".references-count")).toHaveLength(2));
    const linked = host.querySelector(".linked-references")!;
    const unlinked = host.querySelector(".unlinked-references")!;
    const actions = linked.querySelectorAll(".references-header button");
    expect(actions[0].getAttribute("aria-label")).toBe("Copy / export linked references");
    expect(actions[1].getAttribute("aria-label")).toBe("Filter linked references");
    unlinked.querySelector<HTMLElement>(".references-header")!.click();
    unlinked.querySelector<HTMLButtonElement>('[aria-label="Filter unlinked references"]')!.click();
    await vi.waitFor(() => expect(unlinked.querySelectorAll(".ref-filter-chip")).toHaveLength(2));
    unlinked.querySelector<HTMLButtonElement>(".ref-filter-chip")!.click();
    expect(unlinked.querySelector(".references-count")?.textContent).toBe("1");
    expect(linked.querySelector(".references-count")?.textContent).toBe("2");
    const input = unlinked.querySelector<HTMLInputElement>("input[type=search]")!;
    input.value = "banana"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.waitFor(() => expect(unlinked.querySelector(".references-count")?.textContent).toBe("0"));
    expect(linked.querySelector(".references-count")?.textContent).toBe("2");
  } finally { dispose(); }
});
