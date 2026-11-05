import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { appendFeed, loadFeed, pageByName, pinPageWhileDrafting, resetStore } from "../document";
import { installPageContent } from "../document/workingSet";
import { initParser } from "../render/parse";
import { currentDayKey, journalTitle } from "../journal";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { setGraphMeta } from "../graphSession";
import { PageView, reloadJournalsFeedFromStart } from "./Page";
import { graphEpoch } from "../graphSession";
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

const virtualDay = (): PageRead => ({ name: journalTitle(new Date()), title: journalTitle(new Date()), kind: "journal",
  id: "journals/proposed.md", rev: null, pre_block: null,
  blocks: [{ id: "virtual-root", raw: "", children: [], collapsed: false }] });

it.each(["initial", "appended", "host-open", "held"])("a %s virtual journal resolves the file created after the feed was read (GH #863)", async mode => {
  const virtual = virtualDay();
  const physical: PageRead = { ...virtual, id: "journals/external.md", rev: "disk-revision",
    blocks: [{ id: "actual-root", raw: "Body created by an external editor", children: [], collapsed: false }] };
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: [virtual], done: true, next_before_day: null, as_of_day: currentDayKey() });
  vi.spyOn(backend(), "getPage").mockResolvedValue(physical);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => path === physical.id ? physical : null);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <PageView />, root);
  let release: (() => void) | undefined;
  try {
    await vi.waitFor(() => expect(root.querySelector("h1.journal-title")).not.toBeNull());
    // Both initial reads and infinite scroll must admit a logical day, rather
    // than a concrete file that has never existed. All header gestures share it.
    if (mode === "appended") { loadFeed([]); expect(appendFeed([virtual])).toEqual([]); }
    if (mode === "host-open") installPageContent(virtual.name, null, virtual.id);
    if (mode === "held") release = pinPageWhileDrafting(() => virtual.name);
    root.querySelector<HTMLElement>("h1.journal-title")!.click();
    if (mode === "held") {
      await vi.waitFor(() => expect(root.textContent).toContain("unsaved work"));
      expect(pageByName(virtual.name)?.id).toBeUndefined();
      release!(); release = undefined;
    }
    await vi.waitFor(() => expect(root.textContent).toContain("Body created by an external editor"));
    expect(pageByName(virtual.name)?.id).toBe(physical.id);
    expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: virtual.name });
  } finally { release?.(); dispose(); }
});

it("keeps a concrete journal's exact owner after a no-file reply", () => {
  const physical = { ...virtualDay(), rev: "disk-revision" };
  loadFeed([physical]);
  expect(pageByName(physical.name)?.id).toBe(physical.id);
  installPageContent(physical.name, null, "journals/another-proposal.md");
  expect(pageByName(physical.name)?.id).toBe(physical.id);
});
