import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PageView, reloadJournalsFeedFromStart } from "./Page";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { extendFeedForScroll, resetStore, restoreTodayJournalInFeed } from "../document";
import { forgetPage } from "../document/workingSet";
import { resetTabsToJournals } from "../router";
import { currentDayKey, journalTitle, localDayKey } from "../journal";
import { bumpDataRev, graphEpoch, setGraphMeta } from "../graphSession";
import { applyChangeSummary } from "../changeSummary";
import { pageIdentityKey } from "../pageIdentity";
import type { ChangeSummary } from "../backendTypes";
import type { JournalFeedPage, PageRead } from "../types";

beforeAll(initParser);
afterEach(async () => {
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
    pages: [], next_before_day: null, done: true, as_of_day: localDayKey(),
  });
  await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
  resetTabsToJournals();
});

const day = (name: string, id: string): PageRead => ({ id: `journals/${id}.md`, name, title: name, kind: "journal", pre_block: null,
  blocks: [{ id, raw: `Body ${id}`, children: [], collapsed: false }] });

let rev = 1_000;
function publish(fields: Partial<ChangeSummary> = {}) {
  applyChangeSummary({ rev: String(++rev), pages: [], blocks: [], linksTo: [], props: {}, inventoryChanged: false,
    full: false, queryTo: [], days: false, journals: false, templates: false, ...fields });
  bumpDataRev(true);
}

/** Cost audit row 9 (REG-COST-JOURNAL-EDIT-NO-FEED-RESTART, D-20261102-PERF):
 * the host publishes `journals` without `days` for an edit of an existing
 * journal's text, and the Journals feed then keeps its loaded days without a
 * read. `days` (a day reference or planning written) still re-reads it. */
it("a days invalidation reads the loaded window once and replaces its membership", async () => {
  vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
  const today = day(journalTitle(new Date()), "jd-today");
  const january = (date: number) => day(journalTitle(new Date(2000, 0, date)), `jd-${date}`);
  const first: JournalFeedPage = { pages: [today, january(6), january(5)],
    as_of_day: currentDayKey(), done: false, next_before_day: 20000105 };
  const fileless = { ...january(3), rev: null };
  const older = { ...first, pages: [january(4), fileless, january(2)], next_before_day: 20000102 };
  const newDay = january(7);
  const refreshed = { ...older, pages: [today, newDay, ...first.pages.slice(1), january(4), january(2)] };
  const feed = vi.spyOn(backend(), "journalFeedPage").mockImplementation(async (_limit, before, references, through) => {
    if (before === 20000105) return older;
    return references && through === 20000102 ? refreshed : first;
  });
  const host = document.createElement("div"); host.className = "main-content"; document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    const shown = () => [...host.querySelectorAll("[data-feed-day]")].map(el => el.getAttribute("data-feed-day"));
    await vi.waitFor(() => expect(shown()).toEqual(first.pages.map(page => page.name)));
    // The binding's first summary is full by contract; let its refresh settle.
    publish();
    await new Promise(resolve => setTimeout(resolve, 50));
    expect(await extendFeedForScroll()).toBe(true);
    const loaded = [...first.pages, ...older.pages].map(page => page.name);
    expect(shown()).toEqual(loaded);
    const reads = feed.mock.calls.length;
    const pages = [pageIdentityKey(january(4).name)];
    publish({ journals: true, pages });
    await new Promise(resolve => setTimeout(resolve, 50));
    expect(feed.mock.calls.length).toBe(reads);
    expect(shown()).toEqual(loaded);
    // Deleting today's file replaces the feed array with a writable
    // placeholder. The journal window still belongs to the same graph.
    forgetPage(today.name);
    expect(restoreTodayJournalInFeed()).toBeNull();
    expect(shown()).toEqual(loaded);
    publish({ journals: true, days: true, pages });
    await vi.waitFor(() => expect(shown()).toEqual(refreshed.pages.map(page => page.name)));
    expect(feed.mock.calls.slice(reads)).toEqual([[3, null, true, 20000102]]);
  } finally { dispose(); vi.unstubAllGlobals(); }
});
