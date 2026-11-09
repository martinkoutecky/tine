import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PageView, reloadJournalsFeedFromStart } from "./Page";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
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
it("a journal edit re-reads the Journals feed only when its summary fires days", async () => {
  vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
  const today = day(journalTitle(new Date()), "jd-today");
  const first: JournalFeedPage = { pages: [today, day("Jan 3rd, 2000", "jd-3"), day("Jan 2nd, 2000", "jd-2")],
    as_of_day: currentDayKey(), done: true, next_before_day: null };
  const feed = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(first);
  const host = document.createElement("div"); host.className = "main-content"; document.body.append(host);
  const dispose = render(() => <PageView />, host);
  try {
    await vi.waitFor(() => expect(host.querySelectorAll("h1.journal-title")).toHaveLength(3));
    // The binding's first summary is full by contract; let its refresh settle.
    publish();
    await new Promise(resolve => setTimeout(resolve, 50));
    const reads = feed.mock.calls.length;
    publish({ journals: true, pages: [pageIdentityKey("Jan 3rd, 2000")] });
    await new Promise(resolve => setTimeout(resolve, 50));
    expect(feed.mock.calls.length).toBe(reads);
    expect(host.querySelectorAll("h1.journal-title")).toHaveLength(3);
    publish({ journals: true, days: true, pages: [pageIdentityKey("Jan 3rd, 2000")] });
    await vi.waitFor(() => expect(feed.mock.calls.length).toBe(reads + 1));
  } finally { dispose(); vi.unstubAllGlobals(); }
});
