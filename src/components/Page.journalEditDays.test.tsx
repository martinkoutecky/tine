import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PageView, reloadJournalsFeedFromStart } from "./Page";
import { backend, type GraphChange, type GraphAnswersChange } from "../backend";
import { PaneTree } from "../App";
import { initParser } from "../render/parse";
import { applyGraphChange, applyGraphChangesBulk, extendFeedForScroll, pinPageWhileDrafting, resetStore, restoreTodayJournalInFeed } from "../document";
import { forgetPage } from "../document/workingSet";
import { resetTabsToJournals } from "../router";
import { currentDayKey, journalTitle, localDayKey } from "../journal";
import { bumpDataRev, graphEpoch, setGraphMeta } from "../graphSession";
import { applyChangeSummary, setChangeSummaryBaseline } from "../changeSummary";
import { applyHeldExternalChange, heldExternalChangeFor, setConflictPolicyAlwaysAsk } from "../conflictPolicy";
import { layoutRoot, paneRouter, resetPaneLayoutToSingle, restorePaneLayout } from "../panes";
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

describe("GH #987 watcher publications on mounted Journals routes", () => {
  const envelope = (rev: number, days: boolean): GraphAnswersChange => ({
    rev: String(rev), inventoryChanged: false, blockRefCounts: {},
    summary: { rev: String(rev), pages: [], blocks: [], linksTo: [], queryTo: [], props: {}, inventoryChanged: false,
      full: false, days, journals: true, templates: false },
  });
  const event = (page: PageRead): GraphChange => ({
    name: page.name, path: page.id, kind: "journal", created: false, removed: false,
  });
  const drain = () => new Promise<void>(resolve => setTimeout(resolve, 0));

  async function mounted(panes: number) {
    const snapshot = { tabs: [{ history: [{ kind: "journals" as const }], pos: 0, pinned: false }], activeIndex: 0 };
    resetPaneLayoutToSingle(snapshot);
    if (panes === 2) restorePaneLayout(
      { kind: "split", dir: "row", ratio: 0.5, children: [{ kind: "pane", paneId: "main" }, { kind: "pane", paneId: "other" }] },
      new Map([["main", snapshot], ["other", snapshot]]), "main",
    );
    const today = day(journalTitle(new Date()), "watcher-today");
    const older = day(journalTitle(new Date(2000, 0, 2)), "watcher-older");
    const added = day(journalTitle(new Date(2000, 0, 1)), "watcher-added");
    let pages = [today, older];
    const response = (): JournalFeedPage => ({ pages, next_before_day: null, done: true, as_of_day: currentDayKey() });
    const feed = vi.spyOn(backend(), "journalFeedPage").mockImplementation(async () => response());
    vi.spyOn(backend(), "getPage").mockImplementation(async name => pages.find(page => page.name === name) ?? null);
    vi.spyOn(backend(), "setAppBool").mockResolvedValue(undefined);
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <PaneTree node={layoutRoot()} path={[]} />, host);
    await vi.waitFor(() => expect(host.querySelectorAll("[data-feed-day]")).toHaveLength(panes * pages.length));
    await drain();
    feed.mockClear();
    setChangeSummaryBaseline("100");
    return { feed, host, today, older, added, response, dispose, setPages: (next: PageRead[]) => { pages = next; } };
  }

  it.each(["single", "bulk"] as const)("%s plain text updates the loaded journal without a feed read", async route => {
    const ctx = await mounted(1);
    try {
      const changed = { ...ctx.older, blocks: [{ ...ctx.older.blocks[0], raw: "external text" }] };
      ctx.setPages([ctx.today, changed]);
      const answers = envelope(101, false);
      if (route === "single") await applyGraphChange({ ...event(changed), answers });
      else await applyGraphChangesBulk({ changes: [event(changed)], answers });
      await vi.waitFor(() => expect(ctx.host.textContent).toContain("external text"));
      expect(ctx.feed).toHaveBeenCalledTimes(0);
    } finally { ctx.dispose(); }
  });

  it.each(["planning", "create", "remove"] as const)("%s refreshes the shared feed once across two panes", async change => {
    const ctx = await mounted(2);
    try {
      const { today, older, added } = ctx;
      const changed = { ...older, blocks: [{ ...older.blocks[0], raw: "external planning\nSCHEDULED: <2026-12-01 Tue>" }] };
      ctx.setPages(change === "planning" ? [today, changed] : change === "create" ? [today, older, added] : [today]);
      let finish: (() => void) | undefined;
      if (change === "planning") {
        ctx.feed.mockReturnValueOnce(new Promise(resolve => { finish = () => resolve(ctx.response()); }));
      }
      await applyGraphChange({ ...event(change === "create" ? added : older), created: change === "create",
        removed: change === "remove", answers: envelope(101, true) });
      finish?.();
      await vi.waitFor(() => expect(ctx.host.querySelectorAll("[data-feed-day]")).toHaveLength(
        (change === "create" ? 3 : change === "remove" ? 1 : 2) * 2));
      await drain();
      expect(ctx.feed).toHaveBeenCalledTimes(1);
    } finally { ctx.dispose(); }
  });

  it.each([false, true])("one 32-page publication spends only its days read (days: %s)", async days => {
    const ctx = await mounted(1);
    try {
      const changes = [event(ctx.older), event(ctx.today), ...Array.from({ length: 30 }, (_, i) =>
        event(day(journalTitle(new Date(1999, 0, i + 1)), `watcher-burst-${i}`)))];
      if (days) await applyGraphChangesBulk({ changes, answers: envelope(101, days) });
      else {
        // One slow page read in a small publication must not hold up other pages.
        let finish!: (value: PageRead) => void;
        const read = vi.mocked(backend().getPage);
        read.mockClear().mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
        const delivery = applyGraphChangesBulk({ changes, answers: envelope(101, false) });
        try {
          await vi.waitFor(() => expect(read).toHaveBeenCalledWith(ctx.today.name, "journal"));
        } finally { finish(ctx.older); await delivery; }
      }
      await drain();
      expect(ctx.feed).toHaveBeenCalledTimes(days ? 1 : 0);
    } finally { ctx.dispose(); }
  });

  it.each(["single", "bulk"] as const)("%s missing evidence reads the feed once", async route => {
    const ctx = await mounted(1);
    try {
      if (route === "single") await applyGraphChange(event(ctx.older));
      else await applyGraphChangesBulk({ changes: [event(ctx.older)] });
      await drain();
      expect(ctx.feed).toHaveBeenCalledTimes(1);
    } finally { ctx.dispose(); }
  });

  it.each([
    ["draft hold", false, false, 0],
    ["draft hold without a summary", false, true, 1],
    ["Reload from disk", true, false, 0],
    ["Reload from disk without a summary", true, true, 1],
  ] as const)("%s replays the page without replaying the publication", async (_replay, alwaysAsk, missing, reads) => {
    const ctx = await mounted(1);
    let release: (() => void) | undefined;
    try {
      if (alwaysAsk) setConflictPolicyAlwaysAsk(true);
      else release = pinPageWhileDrafting(() => ctx.older.name);
      const changed = { ...ctx.older, blocks: [{ ...ctx.older.blocks[0], raw: "text after replay" }] };
      ctx.setPages([ctx.today, changed]);
      await applyGraphChangesBulk({ changes: [event(changed)], answers: missing ? undefined : envelope(101, false) });
      await drain();
      expect(ctx.host.textContent).not.toContain("text after replay");
      expect(ctx.feed).toHaveBeenCalledTimes(reads);
      if (release) release();
      else {
        expect(heldExternalChangeFor(changed.name)).toBe(true);
        applyHeldExternalChange(changed.name);
      }
      await vi.waitFor(() => expect(ctx.host.textContent).toContain("text after replay"));
      await drain();
      expect(ctx.feed).toHaveBeenCalledTimes(reads);
    } finally { release?.(); setConflictPolicyAlwaysAsk(false); ctx.dispose(); }
  });

  it("coalesces two newer publications into one trailing read", async () => {
    const ctx = await mounted(1);
    try {
      let finish!: (value: JournalFeedPage) => void;
      const first = new Promise<JournalFeedPage>(resolve => { finish = resolve; });
      const old = ctx.response();
      ctx.feed.mockReturnValueOnce(first);
      await applyGraphChange({ ...event(ctx.older), answers: envelope(101, true) });
      expect(ctx.feed).toHaveBeenCalledTimes(1);
      await applyGraphChange({ ...event(ctx.older), answers: envelope(102, true) });
      ctx.setPages([ctx.today, ctx.older, ctx.added]);
      await applyGraphChange({ ...event(ctx.added), created: true, answers: envelope(103, true) });
      expect(ctx.feed).toHaveBeenCalledTimes(1);
      finish(old);
      await vi.waitFor(() => expect(ctx.host.querySelectorAll("[data-feed-day]")).toHaveLength(3));
      await drain();
      expect(ctx.feed).toHaveBeenCalledTimes(2);
    } finally { ctx.dispose(); }
  });

  it("publishes the shared refresh if the first pane leaves Journals", async () => {
    const ctx = await mounted(2);
    try {
      let finish!: (value: JournalFeedPage) => void;
      ctx.feed.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
      ctx.setPages([ctx.today, ctx.older, ctx.added]);
      await applyGraphChange({ ...event(ctx.added), created: true, answers: envelope(101, true) });
      paneRouter("main").openPage("Other");
      finish(ctx.response());
      await vi.waitFor(() => expect(ctx.host.querySelectorAll("[data-feed-day]")).toHaveLength(3));
      expect(ctx.feed).toHaveBeenCalledTimes(1);
    } finally { ctx.dispose(); }
  });
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
