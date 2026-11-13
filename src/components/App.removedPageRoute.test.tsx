import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { backend, type GraphAnswersChange } from "../backend";
import { initParser } from "../render/parse";
import { applyGraphChangesBulk } from "../document/external";
import { ensurePageLoaded, resetStore } from "../document/workingSet";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import type { PageRead } from "../types";
import "../App"; // The application's removal/navigation handler.

beforeAll(() => initParser());
afterEach(() => { resetStore(); resetTabsToJournals(); vi.restoreAllMocks(); });

const removed: PageRead = { name: "Removed", title: "Removed", kind: "page", id: "pages/Removed.md", rev: "1",
  pre_block: null, blocks: [{ id: "removed-block", raw: "gone", collapsed: false, children: [] }] };
const publication = (rev: string, pages: string[]): GraphAnswersChange => ({ rev, inventoryChanged: false, blockRefCounts: {},
  summary: { rev, pages, blocks: [], linksTo: [], props: {}, inventoryChanged: false, full: false } });

// Load-freshness review HOLD on c64fb2fad: the removal handler navigated panes from a route snapshot taken before the
// read, so a reader who had already left the removed page was sent back (goBack) onto it.
it("a missing publication target does not navigate a pane that already left it", async () => {
  ensurePageLoaded(removed);
  mainPaneRouter.openPage("Removed", "page");
  let deliver!: (dto: PageRead | null) => void;
  const read = vi.spyOn(backend(), "getPageByPath").mockImplementationOnce(() => new Promise(resolve => { deliver = resolve; }));
  vi.spyOn(backend(), "getPage").mockResolvedValue(null);
  const pending = applyGraphChangesBulk({ changes: [], answers: publication("1", ["removed"]) });
  await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
  mainPaneRouter.openPage("Elsewhere", "page");
  deliver(null);
  await pending;
  expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Elsewhere" });
});

// Review HOLD on 12bcbeee0: a name-only recheck still sent a reader who had opened another file with the same page name
// back onto the removed file.
it("a missing publication target does not navigate a pane that moved to a same-named file", async () => {
  ensurePageLoaded(removed);
  mainPaneRouter.openFile("pages/Removed.md", "Removed", "page");
  let deliver!: (dto: PageRead | null) => void;
  const read = vi.spyOn(backend(), "getPageByPath").mockImplementationOnce(() => new Promise(resolve => { deliver = resolve; }));
  vi.spyOn(backend(), "getPage").mockResolvedValue(null);
  const pending = applyGraphChangesBulk({ changes: [], answers: publication("1", ["removed"]) });
  await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
  mainPaneRouter.openFile("pages/keeper.md", "Removed", "page");
  deliver(null);
  await pending;
  expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Removed", path: "pages/keeper.md" });
});

it("a pane still showing the missing page leaves it", async () => {
  ensurePageLoaded(removed);
  mainPaneRouter.openPage("Before", "page");
  mainPaneRouter.openPage("Removed", "page");
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  vi.spyOn(backend(), "getPage").mockResolvedValue(null);
  await applyGraphChangesBulk({ changes: [], answers: publication("1", ["removed"]) });
  expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "Before" });
});
