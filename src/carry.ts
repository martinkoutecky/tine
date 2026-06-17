// "Carry unfinished tasks to today" (feature B). The store engine
// (carryUnfinished) does the tree surgery; this orchestrates loading the days it
// needs into the working set, then surfaces the result. Days are passed
// newest→oldest so the newest carried tasks end up on top of today.

import { backend } from "./backend";
import { pageByName, ensurePageLoaded, carryUnfinished, flushPage, isDirty } from "./store";
import { journalTitle } from "./journal";
import { carryKeepsContext, carryHeaderText, pushToast } from "./ui";
import { openJournals } from "./router";
import type { PageDto } from "./types";

async function ensureLoaded(name: string, kind: "journal" | "page"): Promise<boolean> {
  if (pageByName(name)) return true;
  const dto = await backend().getPage(name, kind);
  if (dto) {
    ensurePageLoaded(dto);
    return true;
  }
  return false;
}

/** Make sure today's journal is in the working set (synthesize an empty one if
 *  it has no file yet, like the feed does). */
async function ensureToday(): Promise<string> {
  const t = journalTitle(new Date());
  if (!pageByName(t)) {
    const dto = await backend().getPage(t, "journal");
    const page: PageDto =
      dto ?? { name: t, kind: "journal", title: t, pre_block: null, blocks: [{ id: `new-${t}`, raw: "", collapsed: false, children: [] }] };
    ensurePageLoaded(page);
  }
  return t;
}

// Persist the touched pages to disk NOW, before any feed reload — otherwise
// navigating to journals reloads the (still-old) files and clobbers the move.
async function persist(names: string[]): Promise<void> {
  const seen = new Set<string>();
  await Promise.all(
    names.filter((n) => !seen.has(n) && (seen.add(n), isDirty(n))).map((n) => flushPage(n))
  );
}

async function report(n: number, touched: string[]): Promise<void> {
  await persist(touched);
  openJournals();
  pushToast(n ? `Carried ${n} item${n === 1 ? "" : "s"} to today` : "No unfinished tasks to carry");
}

/** Carry one day's unfinished tasks to today (used from a day's context menu). */
export async function carryDay(pageName: string): Promise<void> {
  const today = await ensureToday();
  if (pageName === today) return;
  if (!(await ensureLoaded(pageName, "journal"))) return;
  const n = carryUnfinished([pageName], carryKeepsContext(), carryHeaderText());
  await report(n, [pageName, today]);
}

/** Carry unfinished tasks from the last `days` days (today−1 … today−days) to
 *  today, newest first. Only days that have a file are touched. */
export async function carryDaysBack(days: number): Promise<void> {
  const today = await ensureToday();
  const base = new Date();
  const titles: string[] = [];
  for (let i = 1; i <= days; i++) {
    const d = new Date(base);
    d.setDate(d.getDate() - i);
    const t = journalTitle(d);
    if (await ensureLoaded(t, "journal")) titles.push(t); // skip days with no file
  }
  const n = carryUnfinished(titles, carryKeepsContext(), carryHeaderText());
  await report(n, [today, ...titles]);
}
