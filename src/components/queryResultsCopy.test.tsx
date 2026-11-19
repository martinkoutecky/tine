import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { closeExportModal, exportModal } from "../ui";
import { openQueryResultsCopy, openReferenceResultsCopy, uniqueResultRoots } from "./queryResultsCopy";
import { bumpGraphEpoch } from "../graphSession";
import type { BlockDto, RefGroup } from "../types";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";
import { initParser } from "../render/parse";
import { ensurePageLoaded, resetStore } from "../document";

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

it.each([64, 128])("reads each child list once across 50 chains of depth %s", depth => {
  let reads = 0;
  const rows: BlockDto[] = [];
  const roots: BlockDto[] = [];
  for (let chain = 0; chain < 50; chain++) {
    let children: BlockDto[] = [];
    for (let level = depth; level > 0; level--) {
      const next = children;
      const row = { ...block(`${chain}-${level}`, ""), get children() { reads++; return next; } };
      rows.push(row);
      children = [row];
    }
    roots.push(children[0]);
  }
  const deduped = uniqueResultRoots(rows);
  expect(reads).toBe(50 * depth);
  expect(deduped.map(row => row.id)).toEqual(roots.map(row => row.id));
});

it.each([true, false])("uses the current locator even when an authored ID shadows it (loaded=%s)", async loaded => {
  const row = block("positional", "Original reference");
  const source = { id: "pages/Source.md", name: "Source", title: "Source", kind: "page" as const, rev: "original", pre_block: null,
    blocks: [{ ...row, children: [block("child", "Original child")] }, block("shadow", "Unrelated row\nid:: positional")] };
  if (loaded) ensurePageLoaded(source);
  vi.spyOn(backend(), "getPage").mockResolvedValue(source);
  const groups: RefGroup[] = [{ page: "Source", kind: "page", export_source: { path: source.id, rev: "original" }, blocks: [row] }];
  expect(await openReferenceResultsCopy(groups, () => true)).toBe(true);
  expect(JSON.stringify(exportModal())).toContain("Original child");
  expect(JSON.stringify(exportModal())).not.toContain("Unrelated row");
});

it("refuses a by-name read that answered from a different physical owner", async () => {
  const row = block("positional", "Original reference");
  vi.spyOn(backend(), "getPage").mockResolvedValue({
    id: "pages/twin/Source.md", name: "Source", title: "Source", kind: "page", rev: "original", pre_block: null,
    blocks: [{ ...row, children: [block("twin-child", "Unrelated subtree")] }],
  });
  const groups: RefGroup[] = [{ page: "Source", kind: "page",
    export_source: { path: "pages/Source.md", rev: "original" }, blocks: [row] }];
  expect(await openReferenceResultsCopy(groups, () => true)).toBe(false);
  expect(exportModal()).toBeNull();
});

it.each([
  [true, "unique"], [false, "unique"],
  [true, "duplicate current"], [false, "duplicate current"],
  [true, "duplicate original"], [false, "duplicate original"],
] as const)("follows only a unique authored ID across revisions (loaded=%s, %s)", async (loaded, claim) => {
  const raw = "Reference\nid:: authored";
  const original = { ...block("old-position", raw), has_id: true, properties: [["id", "authored"]] as [string, string][] };
  const replacement = block("new-position", raw, [block("source-child", "Original subtree moved here")]);
  const distractor = block("old-position", claim === "duplicate current" ? raw : "Unrelated reuse of old position");
  const source = { id: "pages/Source.md", name: "Source", title: "Source", kind: "page" as const, rev: "changed", pre_block: null,
    blocks: [replacement, distractor] };
  if (loaded) ensurePageLoaded(source);
  vi.spyOn(backend(), "getPage").mockResolvedValue(source);
  const groups: RefGroup[] = [{ page: "Source", kind: "page",
    export_source: { path: source.id, rev: "original", ambiguous_ids: claim === "duplicate original" ? ["authored"] : [] },
    blocks: [original] }];
  expect(await openReferenceResultsCopy(groups, () => true)).toBe(claim === "unique");
  if (claim === "unique") {
    expect(JSON.stringify(exportModal())).toContain("Original subtree moved here");
    expect(JSON.stringify(exportModal())).not.toContain("Unrelated reuse");
  } else {
    expect(exportModal()).toBeNull();
  }
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
