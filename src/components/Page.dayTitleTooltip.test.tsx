import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore } from "../document";
import { initParser } from "../render/parse";
import { currentDayKey, journalTitle } from "../journal";
import { resetTabsToJournals } from "../router";
import { graphEpoch, setGraphMeta } from "../graphSession";
import { PageView, reloadJournalsFeedFromStart } from "./Page";
import type { PageRead } from "../types";

beforeAll(initParser);
afterEach(async () => {
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: [], done: true, next_before_day: null, as_of_day: currentDayKey() });
  await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
  resetTabsToJournals();
});

/** D-20261119-DRM: a day is renamed (its notes moved to a page) by double-clicking
 * its title, as a page is; the title's tooltip says so, since the day menu has no row. */
it("a day's title tooltip says it can be renamed by double-clicking", async () => {
  const name = journalTitle(new Date());
  const day: PageRead = { name, title: name, kind: "journal", id: "journals/today.md", rev: "r1", pre_block: null,
    blocks: [{ id: "b1", raw: "Body", children: [], collapsed: false }] };
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: [day], done: true, next_before_day: null, as_of_day: currentDayKey() });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector("h1.journal-title")).not.toBeNull());
    expect(root.querySelector("h1.journal-title")!.getAttribute("title")).toMatch(/^Double-click to rename/);
  } finally { dispose(); }
});
