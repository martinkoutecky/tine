import { describe, expect, it } from "vitest";
import type { PageEntry } from "./types";
import { arrangeAllPages, DEFAULT_SORT, formatPageDay, formatUpdated, listablePages, nextSort, visibleRange } from "./allPagesList";
import { parsePersistedSession } from "./session";
import { routeTitle, sameRoute } from "./router";

// GH #335: the list logic of the All Pages view. Pure: no backend, no DOM.

const page = (name: string, path = `pages/${name}.md`): PageEntry => ({ name, path, kind: "page", date_key: null });
const journal = (day: number): PageEntry => ({ name: `J ${day}`, path: `journals/${day}.md`, kind: "journal", date_key: day });
const TITLE = { key: "title", desc: false } as const;
const names = (rows: { entry: PageEntry }[]) => rows.map((row) => row.entry.name);

describe("All Pages list", () => {
  const pages = [page("banana"), page("Apple"), page("cherry 10"), page("cherry 9"), journal(20260102), journal(20260301)];

  it("lists pages only until journals are asked for", () => {
    expect(listablePages(pages, false).map((p) => p.kind)).toEqual(["page", "page", "page", "page"]);
    expect(listablePages(pages, true)).toHaveLength(6);
  });

  it("sorts titles case-insensitively with numbers in numeric order, both ways", () => {
    const shown = listablePages(pages, false);
    expect(names(arrangeAllPages(shown, "", TITLE))).toEqual(["Apple", "banana", "cherry 9", "cherry 10"]);
    expect(names(arrangeAllPages(shown, "", { key: "title", desc: true }))).toEqual(["cherry 10", "cherry 9", "banana", "Apple"]);
  });

  it("sorts by journal day, newest first, and keeps day-less pages after every journal in both directions", () => {
    expect(names(arrangeAllPages(pages, "", { key: "day", desc: true }))).toEqual([
      "J 20260301", "J 20260102", "Apple", "banana", "cherry 9", "cherry 10",
    ]);
    expect(names(arrangeAllPages(pages, "", { key: "day", desc: false })).slice(0, 2)).toEqual(["J 20260102", "J 20260301"]);
    expect(names(arrangeAllPages(pages, "", { key: "day", desc: false })).slice(2)).toEqual(["Apple", "banana", "cherry 9", "cherry 10"]);
  });

  it("filters by name substring, case-insensitively, and an unmatched filter is empty", () => {
    expect(names(arrangeAllPages(pages, "  CHERRY ", TITLE))).toEqual(["cherry 9", "cherry 10"]);
    expect(arrangeAllPages(pages, "zzz", TITLE)).toEqual([]);
  });

  it("disambiguates two pages with one name by folder, and keeps both rows", () => {
    const rows = arrangeAllPages([page("Same", "pages/a/Same.md"), page("Same", "pages/b/Same.md")], "", TITLE);
    expect(rows.map((row) => row.label)).toEqual(["Same — a/", "Same — b/"]);
  });

  it("the first click on a column picks its natural direction and the second flips it", () => {
    const byDay = nextSort(TITLE, "day");
    expect(byDay).toEqual({ key: "day", desc: true });
    expect(nextSort(byDay, "day")).toEqual({ key: "day", desc: false });
    expect(nextSort(byDay, "title")).toEqual({ key: "title", desc: false });
  });

  it("opens newest-updated first, like OG's All pages", () => {
    expect(DEFAULT_SORT).toEqual({ key: "updated", desc: true });
    const modified = new Map([["pages/banana.md", 300], ["pages/Apple.md", 100], ["pages/cherry 9.md", 200]]);
    const shown = listablePages(pages, false);
    expect(names(arrangeAllPages(shown, "", DEFAULT_SORT, undefined, modified))).toEqual(["banana", "cherry 9", "Apple", "cherry 10"]);
    expect(names(arrangeAllPages(shown, "", { key: "updated", desc: false }, undefined, modified))).toEqual(["Apple", "cherry 9", "banana", "cherry 10"]);
    expect(arrangeAllPages(shown, "", DEFAULT_SORT, undefined, modified)[0].modified).toBe(300);
  });

  it("puts a page with no recorded time last in both directions, ordered by name", () => {
    const shown = [page("zed"), page("Alpha"), page("timed")];
    const modified = new Map([["pages/timed.md", 5]]);
    expect(names(arrangeAllPages(shown, "", DEFAULT_SORT, undefined, modified))).toEqual(["timed", "Alpha", "zed"]);
    expect(names(arrangeAllPages(shown, "", { key: "updated", desc: false }, undefined, modified))).toEqual(["timed", "Alpha", "zed"]);
  });

  it("the Updated column's first click is newest-first, the title column's is A to Z", () => {
    expect(nextSort(TITLE, "updated")).toEqual({ key: "updated", desc: true });
    expect(nextSort({ key: "updated", desc: true }, "updated")).toEqual({ key: "updated", desc: false });
    expect(nextSort(DEFAULT_SORT, "title")).toEqual({ key: "title", desc: false });
  });

  it("shows an update time as local date and minutes, and a missing one as a dash", () => {
    const at = new Date(2026, 5, 15, 14, 5).getTime() / 1000;
    expect(formatUpdated(at)).toBe("2026-06-15 14:05");
    expect(formatUpdated(undefined)).toBe("—");
    expect(formatUpdated(Number.NaN)).toBe("—");
  });

  it("shows a journal day as a date and a page without one as a dash, never a zero date", () => {
    expect(formatPageDay(20260312)).toBe("2026-03-12");
    expect(formatPageDay(null)).toBe("—");
  });
});

describe("All Pages window", () => {
  it("covers the viewport plus overscan and never more than the list", () => {
    expect(visibleRange(0, 640, 32, 10_000, 8)).toEqual({ start: 0, end: 28 });
    expect(visibleRange(32 * 5000, 640, 32, 10_000, 8)).toEqual({ start: 4992, end: 5028 });
    expect(visibleRange(32 * 9990, 640, 32, 10_000, 8)).toEqual({ start: 9982, end: 10_000 });
    expect(visibleRange(0, 640, 32, 0, 8)).toEqual({ start: 0, end: 0 });
  });
});

describe("the all-pages route", () => {
  it("is titled and compared like the other one-per-kind views", () => {
    expect(routeTitle({ kind: "all-pages" })).toBe("All pages");
    expect(sameRoute({ kind: "all-pages" }, { kind: "all-pages" })).toBe(true);
    expect(sameRoute({ kind: "all-pages" }, { kind: "conflicts" })).toBe(false);
  });

  it("survives a session restore", () => {
    const raw = JSON.stringify({ tabs: [{ history: [{ kind: "journals" }, { kind: "all-pages" }], pos: 1, pinned: false }], activeIndex: 0 });
    expect(parsePersistedSession(raw)?.snapshots.get("main")?.tabs[0].history).toEqual([{ kind: "journals" }, { kind: "all-pages" }]);
  });
});
