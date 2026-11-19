import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { closeExportModal, exportModal } from "../ui";
import { openQueryResultsCopy } from "./queryResultsCopy";
import { bumpGraphEpoch } from "../graphSession";
import type { BlockDto, RefGroup } from "../types";
import { adoptResults, captureResultsTicket, resultsSettled } from "../queryResultsEpoch";
import { initParser } from "../render/parse";
import { beginPageHeaderEdit, ensurePageLoaded, resetStore } from "../document";

beforeAll(initParser);

const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, children, collapsed: false });
afterEach(() => { vi.restoreAllMocks(); closeExportModal(); resetStore(); });

it.each(["md", "org"] as const)("copies the %s preamble itself before and after source hydration", async format => {
  const source = { id: `pages/Copy.${format}`, name: "Copy", title: "Copy", kind: "page" as const, format,
    pre_block: (format === "md" ? "related:: [[Target]]" : "#+RELATED: [[Target]]") + "\n\nProse [[Target]]\n",
    blocks: [block("ordinary", "Unrelated outline")] };
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(source);
  for (const loaded of [false, true]) {
    if (loaded) ensurePageLoaded(source);
    const groups: RefGroup[] = [{ page: "Copy", kind: "page", path: source.id,
      blocks: [{ ...block("page-preamble:Page:copy", "Prose [[Target]]"), page_preamble: true }] }];
    adoptResults(groups, captureResultsTicket());
    await openQueryResultsCopy(groups);
    const modal = exportModal();
    expect(modal && "nodes" in modal && modal.nodes).toMatchObject([{ raw: "Copy", children: [{ raw: "Prose [[Target]]", children: [] }] }]);
    expect(JSON.stringify(modal)).not.toContain("Unrelated outline");
    closeExportModal();
  }
});

it("copies duplicate matched descendants only once under the journal date title", async () => {
  const child = block("c", "Child");
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue({ id: "journals/2026_10_13.md", name: "Oct 13th, 2026", title: "Oct 13th, 2026", kind: "journal", pre_block: null,
    blocks: [block("p", "Parent", [child])] });
  const groups: RefGroup[] = [{ page: "Oct 13th, 2026", kind: "journal", path: "journals/2026_10_13.md", blocks: [block("p", "Parent", [child]), child] }];
  adoptResults(groups, captureResultsTicket());
  await openQueryResultsCopy(groups);
  const modal = exportModal();
  expect(modal && "nodes" in modal && modal.nodes).toMatchObject([{ raw: "Oct 13th, 2026", format: "md", children: [
    { raw: "Parent", format: "md", children: [{ raw: "Child", format: "md", children: [] }] },
  ] }]);
});

it("hydrates the selected source subtree without copying unrelated roots", async () => {
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue({ id: "pages/Copy.md", name: "Copy", title: "Copy", kind: "page", pre_block: null,
    blocks: [block("p", "Parent", [block("c", "Child")]), block("x", "Unrelated")] });
  const groups: RefGroup[] = [{ page: "Copy", kind: "page", path: "pages/Copy.md", blocks: [block("p", "Parent")] }];
  adoptResults(groups, captureResultsTicket());
  await openQueryResultsCopy(groups);
  const modal = exportModal();
  expect(modal && "nodes" in modal && modal.nodes[0].children[0].children[0].raw).toBe("Child");
  expect(JSON.stringify(modal)).not.toContain("Unrelated");
});

it("does not open a copy modal after switching graph during source hydration", async () => {
  let finish!: (value: null) => void;
  vi.spyOn(backend(), "getPageByPath").mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const pending = openQueryResultsCopy([{ page: "Copy", kind: "page", path: "pages/Copy.md", blocks: [block("p", "Parent")] }]);
  bumpGraphEpoch(); finish(null); await pending;
  expect(exportModal()).toBeNull();
});

// D-20261119-PG, review da945bc19 (AR-1): a result's synthetic rows are checked
// against where the source's header ends, the one answer `pageOpening` gives,
// so later property groups and an open header editor do not turn unchanged
// rows into "results changed".
it.each([["unloaded"], ["loaded"], ["loaded, header open"]] as const)("copies the header and opening rows of a page with later property groups (%s)", async state => {
  const header = "related:: [[Target]]";
  const opening = "later:: x\nlast:: y\n\nProse [[Target]]";
  const source = { id: "pages/Groups.md", name: "Groups", title: "Groups", kind: "page" as const, format: "md" as const,
    pre_block: `${header}\n\n${opening}\n`, blocks: [block("ordinary", "Unrelated outline")] };
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(source);
  if (state !== "unloaded") ensurePageLoaded(source);
  const groups: RefGroup[] = [{ page: "Groups", kind: "page", path: source.id, blocks: [
    { ...block("page-property:Page:groups", header), page_property: true },
    { ...block("page-preamble:Page:groups", opening), page_preamble: true },
  ] }];
  // The user opens the source's header editor; the query's answer is adopted
  // once that settles, as a rerun adopts it.
  if (state === "loaded, header open") {
    expect(beginPageHeaderEdit("Groups")).toBeTruthy();
    await vi.waitFor(() => expect(resultsSettled()).toBe(true));
  }
  adoptResults(groups, captureResultsTicket());
  await openQueryResultsCopy(groups);
  const modal = exportModal();
  expect(modal && "nodes" in modal && modal.nodes).toMatchObject([{ raw: "Groups", children: [
    { raw: expect.stringContaining("related:: [[Target]]") }, { raw: opening, children: [] },
  ] }]);
});
