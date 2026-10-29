import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PageView, reloadJournalsFeedFromStart } from "./Page";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { extendFeedForScroll } from "../document";
import { resetTabsToJournals } from "../router";
import { currentDayKey, journalTitle, localDayKey } from "../journal";
import { graphEpoch, setGraphMeta } from "../graphSession";
import { startEditing, endEdit } from "../editorController";
import type { JournalFeedPage, PageRead } from "../types";

beforeAll(initParser);
afterEach(async () => {
  endEdit("page-navigation");
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
    pages: [], next_before_day: null, done: true, as_of_day: localDayKey(),
  });
  await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
  resetTabsToJournals();
});

const day = (name: string, id: string): PageRead => ({ id: `journals/${id}.md`, name, title: name, kind: "journal", pre_block: null,
  blocks: [{ id, raw: `Body ${id}`, children: [], collapsed: false }] });

/** JOURNAL-ENOENT R2.3 (REG-JOURNAL-FEED-PAGING-WHILE-EDITING): a restart
 * deferred because the caret is in a feed day kept every scroll from paging:
 * loadMore retried the restart, which deferred again, so the feed stayed at
 * its first three days for as long as today's journal had the caret. */
it("scrolling pages older days while a deferred feed restart waits for the editor", async () => {
  vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
  const today = day(journalTitle(new Date()), "fp-today");
  const first: JournalFeedPage = { pages: [today, day("Jan 3rd, 2000", "fp-3"), day("Jan 2nd, 2000", "fp-2")],
    as_of_day: currentDayKey(), done: false, next_before_day: 10_957 };
  const older: JournalFeedPage = { pages: [day("Jan 1st, 2000", "fp-1")], as_of_day: currentDayKey(), done: true, next_before_day: null };
  const feed = vi.spyOn(backend(), "journalFeedPage").mockImplementation(async (_limit, before) => before == null ? first : older);
  const diagnostic = vi.spyOn(backend(), "diagnosticFrontendEvent").mockResolvedValue();
  const host = document.createElement("div"); host.className = "main-content"; document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.querySelectorAll("h1.journal-title")).toHaveLength(3));
    startEditing("fp-today");
    // A watcher/full publication restart arrives while today has the caret.
    await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
    expect(diagnostic).toHaveBeenCalledWith("journal_feed", { journalFeedReason: "active-edit" });
    const reads = feed.mock.calls.length;
    await extendFeedForScroll();
    await vi.waitFor(() => expect(host.querySelectorAll("h1.journal-title")).toHaveLength(4));
    expect(feed.mock.calls.slice(reads).map(([, before]) => before)).toEqual([10_957]);
  } finally { dispose(); }
});
