// REG-CM-YEARLESS-DAY-KEY (round 3): under a year-less title format (`MM-dd`)
// the store names a day's page by the spelling that identifies it
// (`journalHandle`, `2026_11_02`) and titles it by its label (`11-02`). The
// app keys its feed, today and routes by that name and shows the label.
import { afterEach, beforeAll, expect, it } from "vitest";
import { initParser } from "./render/parse";
import { appNow, journalHandle, journalTitle, pageLabel, setJournalTitleFormat } from "./journal";
import { loadFeed, pageByName, resetStore, withToday } from "./document";
import { doc } from "./document/model";
import { routeTitle } from "./router";
import type { PageDto } from "./types";

beforeAll(initParser);
afterEach(() => { setJournalTitleFormat(null); resetStore(); });

/** A day's page as the store sends it: named by its handle, titled by its label. */
const day = (date: Date, raw: string): PageDto => ({
  name: journalHandle(date), title: journalTitle(date), kind: "journal", pre_block: null,
  blocks: [{ id: `${journalHandle(date)}-b`, raw, collapsed: false, children: [] }],
} as PageDto);
const yearAgo = () => { const now = appNow(); return new Date(now.getFullYear() - 1, now.getMonth(), now.getDate()); };

it("the feed keeps two years of one MM-dd day apart and finds today among them", () => {
  setJournalTitleFormat("MM-dd");
  const today = day(appNow(), "today"), last = day(yearAgo(), "last year");
  expect(today.title).toBe(last.title);
  loadFeed(withToday([today, last]));
  expect(doc.feed).toEqual([today.name, last.name]);
  expect(pageByName(today.name)!.roots.map((id) => doc.byId[id].raw)).toEqual(["today"]);
  expect(pageByName(last.name)!.roots.map((id) => doc.byId[id].raw)).toEqual(["last year"]);
});

it("last year's MM-dd day is not taken for today: the feed opens today above it", () => {
  setJournalTitleFormat("MM-dd");
  const last = day(yearAgo(), "last year");
  loadFeed(withToday([last]));
  expect(doc.feed).toEqual([journalHandle(appNow()), last.name]);
  expect(pageByName(journalHandle(appNow()))!.title).toBe(journalTitle(appNow()));
});

it("a page named by a day's handle is shown by the day's label; other names as they are", () => {
  setJournalTitleFormat("MM-dd");
  const handle = journalHandle(new Date(2026, 10, 2));
  expect(handle).not.toBe("11-02");
  expect(pageLabel(handle)).toBe("11-02");
  expect(routeTitle({ kind: "page", name: handle, pageKind: "journal" } as any)).toBe("11-02");
  for (const name of ["11-02", "Meeting", "2026-11-02 notes"]) expect(pageLabel(name)).toBe(name);
  setJournalTitleFormat(null);
  const title = journalTitle(new Date(2026, 10, 2));
  expect(journalHandle(new Date(2026, 10, 2))).toBe(title);
  expect(pageLabel(title)).toBe(title);
});
