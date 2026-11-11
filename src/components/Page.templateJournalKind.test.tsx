// GH #863 round 6 (UI-GH863-TEMPLATE-JOURNAL-KIND): with a default journals
// template, Tine creates today's file before the feed has loaded today's page.
// The page host's no-file answer installed that page as an ordinary page, and
// later reads of the same bytes kept the guess, so the title's route carried
// kind "page" and the fresh disk read (kind "journal") was refused as
// "no longer available at that path" until an external edit changed the bytes.

import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { loadFeed, pageByName, resetStore } from "../document";
import { loadRoutedPage } from "../document/workingSet";
import { createPage } from "../document/host/wiring";
import { bindTestHost } from "../document/host/wiring.test.support";
import { initParser } from "../render/parse";
import { currentDayKey, journalTitle } from "../journal";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { graphEpoch, setGraphMeta } from "../graphSession";
import { PageView, reloadJournalsFeedFromStart } from "./Page";
import type { PageDto, PageRead } from "../types";

beforeAll(initParser);
beforeEach(async () => { resetStore(); await bindTestHost(); });
afterEach(async () => {
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: [], done: true, next_before_day: null, as_of_day: currentDayKey() });
  await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
  resetTabsToJournals();
});

const today = () => journalTitle(new Date());
const templated = (): PageDto => ({ name: today(), kind: "journal", title: today(), pre_block: null,
  blocks: [{ id: "template-root", raw: "GH863 template note", collapsed: false, children: [] }] });

it("a journal day the template created is held as a journal day", async () => {
  await createPage(today(), templated());
  expect(pageByName(today())).toMatchObject({ kind: "journal" });
});

it("a read of the same bytes corrects a held guess of the file's kind", () => {
  const path = "journals/2026_01_02.md";
  const body = [{ id: "guess-root", raw: "Day body", collapsed: false, children: [] }];
  loadRoutedPage({ name: "2026-01-02", kind: "page", title: "2026-01-02", id: path, rev: null, pre_block: null, blocks: body });
  loadFeed([{ name: "2026-01-02", kind: "journal", title: "2026-01-02", id: path, rev: "disk-revision", pre_block: null,
    blocks: [{ ...body[0], id: "disk-root" }] }]);
  expect(pageByName("2026-01-02")).toMatchObject({ kind: "journal", id: path });
  // A guess (no revision) never overrides the file's answer.
  loadRoutedPage({ name: "2026-01-02", kind: "page", title: "2026-01-02", id: path, pre_block: null, blocks: body });
  expect(pageByName("2026-01-02")).toMatchObject({ kind: "journal" });
});

it("clicking the title of today's template-created journal opens it (GH #863)", async () => {
  await createPage(today(), templated());
  const path = pageByName(today())!.id!;
  expect(path).toBeTruthy();
  const file: PageRead = { ...templated(), id: path, rev: "disk-revision",
    blocks: [{ id: "disk-root", raw: "GH863 template note", collapsed: false, children: [] }] };
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: [file], done: true, next_before_day: null, as_of_day: currentDayKey() });
  vi.spyOn(backend(), "getPage").mockResolvedValue(file);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async p => p === path ? file : null);
  await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector("h1.journal-title")).not.toBeNull());
    root.querySelector<HTMLElement>("h1.journal-title")!.click();
    await vi.waitFor(() => expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: today(), pageKind: "journal" }));
    await vi.waitFor(() => expect(root.textContent).toContain("GH863 template note"));
    expect(root.textContent).not.toContain("no longer available");
    expect(root.querySelector("h1.journal-title")).not.toBeNull();
  } finally { dispose(); }
});

it("a route that guessed the wrong kind for a file opens it as the file's kind", async () => {
  const path = "journals/2026_01_02.md";
  const file: PageRead = { name: "2026-01-02", kind: "journal", title: "2026-01-02", id: path, rev: "disk-revision",
    pre_block: null, blocks: [{ id: "disk-root", raw: "Day body", collapsed: false, children: [] }] };
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async p => p === path ? file : null);
  mainPaneRouter.openPageTarget({ name: file.name, pageKind: "page", path });
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("Day body"));
    expect(root.textContent).not.toContain("no longer available");
    expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: file.name, pageKind: "journal", path });
  } finally { dispose(); }
});
