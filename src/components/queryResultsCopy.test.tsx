import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { closeExportModal, exportModal } from "../ui";
import { openQueryResultsCopy } from "./queryResultsCopy";
import { bumpGraphEpoch } from "../graphSession";
import type { BlockDto, RefGroup } from "../types";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";

const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, children, collapsed: false });
afterEach(() => { vi.restoreAllMocks(); closeExportModal(); });

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
