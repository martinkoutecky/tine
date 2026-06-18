// "Carry unfinished tasks to today" (feature B). The store engine
// (carryUnfinished) does the tree surgery; this orchestrates loading the days it
// needs into the working set, then surfaces the result. Days are passed
// newest→oldest so the newest carried tasks end up on top of today.

import { backend } from "./backend";
import { pageByName, ensurePageLoaded, carryUnfinished, flushPage, isDirty, markDirty } from "./store";
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
// Returns whether every dirty touched page actually saved.
// Persist `today` (the ADDITION side) FIRST and only flush the source days once it
// lands — so a today-conflict can't leave the carried blocks removed from their
// source files but never written to today (a removal-only, data-losing state).
async function persist(today: string, sources: string[]): Promise<boolean> {
  // Destination (today) must land first. carryUnfinished intentionally left the
  // source days NOT dirty, so nothing can save a source removal until today is
  // safely written — only THEN do we mark + flush the sources.
  if (isDirty(today) && !(await flushPage(today))) return false;
  const uniq = [...new Set(sources)].filter((n) => n !== today);
  for (const n of uniq) markDirty(n);
  const results = await Promise.all(uniq.map((n) => flushPage(n)));
  return results.every(Boolean);
}

async function report(n: number, today: string, sources: string[]): Promise<void> {
  // If a touched page couldn't be saved (conflict / disk error), DON'T reload the
  // journals feed — that would re-read the old files and drop the carried blocks
  // from memory. Leave the move in memory and surface the failure.
  if (!(await persist(today, sources))) {
    pushToast("Carry couldn't be saved — resolve the conflict; your moved tasks are kept in the editor.", "error");
    return;
  }
  openJournals();
  pushToast(n ? `Carried ${n} item${n === 1 ? "" : "s"} to today` : "No unfinished tasks to carry");
}

/** Carry unfinished tasks from the previous *non-empty* day to today. "Previous
 *  day" means the most recent journal before today that actually has content
 *  (not literally yesterday, which is often blank). */
export async function carryPrevDay(): Promise<void> {
  const today = new Date();
  const todayKey =
    today.getFullYear() * 10000 + (today.getMonth() + 1) * 100 + today.getDate();
  let days: number[] = [];
  try {
    days = await backend().journalContentDays();
  } catch {
    days = [];
  }
  const prevKey = days.filter((k) => k < todayKey).sort((a, b) => a - b).pop();
  if (prevKey == null) {
    pushToast("No previous day with content to carry from");
    return;
  }
  const d = new Date(Math.floor(prevKey / 10000), (Math.floor(prevKey / 100) % 100) - 1, prevKey % 100);
  await carryDay(journalTitle(d));
}

/** Carry one day's unfinished tasks to today (used from a day's context menu). */
export async function carryDay(pageName: string): Promise<void> {
  const today = await ensureToday();
  if (pageName === today) return;
  if (!(await ensureLoaded(pageName, "journal"))) return;
  const n = carryUnfinished([pageName], carryKeepsContext(), carryHeaderText());
  await report(n, today, [pageName]);
}

/** Carry unfinished tasks from the last `days` days (today−1 … today−days) to
 *  today, newest first. Only days that have a file are touched. */
export async function carryDaysBack(days: number): Promise<void> {
  const today = await ensureToday();
  const base = new Date();
  const candidates: string[] = [];
  for (let i = 1; i <= days; i++) {
    const d = new Date(base);
    d.setDate(d.getDate() - i);
    candidates.push(journalTitle(d));
  }
  // Load all the day files in parallel rather than one IPC round-trip at a time.
  const loaded = await Promise.all(candidates.map((t) => ensureLoaded(t, "journal")));
  const titles = candidates.filter((_, i) => loaded[i]); // skip days with no file
  const n = carryUnfinished(titles, carryKeepsContext(), carryHeaderText());
  await report(n, today, titles);
}
