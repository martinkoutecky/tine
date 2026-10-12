import type { PageEntry } from "./types";
import { pageListLabels } from "./pages";

// The All Pages view's list logic (GH #335): which rows, in what order, and which
// slice of them is on screen. Pure functions over the one shared page index
// (`pageIndex.ts`); nothing here reads the backend, so a sort or a filter costs
// no IPC and a row costs no call (I-22).

export type AllPagesSortKey = "title" | "day";
export interface AllPagesSort {
  key: AllPagesSortKey;
  desc: boolean;
}
export interface AllPagesRow {
  entry: PageEntry;
  /** The name, disambiguated with its folder when two pages share it. */
  label: string;
}

/** The first-click direction of a column: names ascend, journal days start newest-first. */
export const DEFAULT_SORT: AllPagesSort = { key: "title", desc: false };

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/** Clicking the active column flips it; clicking another column starts it in its natural direction. */
export function nextSort(current: AllPagesSort, key: AllPagesSortKey): AllPagesSort {
  if (current.key === key) return { key, desc: !current.desc };
  return { key, desc: key === "day" };
}

/** `20260312` shown as `2026-03-12`. A page without a journal day shows `—`: absent is not a date. */
export function formatPageDay(day: number | null | undefined): string {
  if (day === null || day === undefined || !Number.isFinite(day)) return "—";
  const text = String(Math.trunc(day)).padStart(8, "0");
  return `${text.slice(0, 4)}-${text.slice(4, 6)}-${text.slice(6, 8)}`;
}

/** Rows of the inventory that this view lists. Pages always; journals only when asked,
 *  as in Logseq's All pages (whose journals toggle starts off). O(pages). */
export function listablePages(pages: readonly PageEntry[], includeJournals: boolean): PageEntry[] {
  return includeJournals ? pages.slice() : pages.filter((page) => page.kind === "page");
}

/** Filter by name (case-insensitive substring) then sort. A page without a day sorts
 *  after every journal in both directions of the day column. Ties keep path order, so
 *  the order is total and stable. O(k log k) for the k kept pages. `label` is the
 *  disambiguator over ALL listed pages (`pageListLabels`): the caller builds it once
 *  per listing so a keystroke in the filter does not rescan every page. */
export function arrangeAllPages(
  pages: readonly PageEntry[],
  filter: string,
  sort: AllPagesSort,
  label: (page: PageEntry) => string = pageListLabels(pages as PageEntry[]),
): AllPagesRow[] {
  const needle = filter.trim().toLowerCase();
  const kept = needle ? pages.filter((page) => page.name.toLowerCase().includes(needle)) : pages.slice();
  const direction = sort.desc ? -1 : 1;
  kept.sort((a, b) => {
    if (sort.key === "day") {
      const aHas = a.date_key !== null, bHas = b.date_key !== null;
      if (aHas !== bHas) return aHas ? -1 : 1;
      if (aHas && a.date_key !== b.date_key) return ((a.date_key as number) - (b.date_key as number)) * direction;
    }
    const byName = collator.compare(a.name, b.name);
    if (byName !== 0) return (sort.key === "title" ? direction : 1) * byName;
    return a.path < b.path ? -1 : a.path > b.path ? 1 : 0;
  });
  return kept.map((entry) => ({ entry, label: label(entry) }));
}

/** The slice of a fixed-row-height list that must exist in the DOM. `overscan` rows
 *  on each side keep a fast scroll from flashing blanks. Total work is O(1). */
export function visibleRange(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  total: number,
  overscan = 8,
): { start: number; end: number } {
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const last = Math.ceil((Math.max(0, scrollTop) + Math.max(0, viewportHeight)) / rowHeight);
  return {
    start: Math.max(0, Math.min(total, first - overscan)),
    end: Math.max(0, Math.min(total, last + overscan)),
  };
}
