// "Carry unfinished tasks to today" (feature B). The store engine
// (carryUnfinished) does the tree surgery; this orchestrates loading the days it
// needs into the working set, then surfaces the result. Days are passed
// newest→oldest so the newest carried tasks end up on top of today.

import { backend } from "./backend";
import { graphOwner, readOwned, type Owner } from "./owned";
import { pageByName, ensurePageLoaded, carryUnfinished, flushPage, carryTodayPage, refuseConflictedMove } from "./document";
import { journalTitle } from "./journal";
import { carryKeepsContext, carryHeaderText } from "./ui";
import { pushToast } from "./toasts";
import { openJournals } from "./router";

async function ensureLoaded(name: string, kind: "journal" | "page", owner: Owner): Promise<boolean> {
  if (pageByName(name)) return true;
  const result = await readOwned(owner, backend().getPage(name, kind));
  if (result.kind === "stale") return false;
  if (result.value) {
    ensurePageLoaded(result.value);
    return true;
  }
  return false;
}

/** Make sure today's journal is in the working set (synthesize an empty one if
 *  it has no file yet, like the feed does). */
async function ensureToday(owner: Owner): Promise<string | null> {
  const t = journalTitle(new Date());
  if (!pageByName(t)) {
    const result = await readOwned(owner, backend().getPage(t, "journal"));
    if (result.kind === "stale") return null;
    const page = result.value ?? carryTodayPage(t);
    ensurePageLoaded(page);
  }
  return t;
}

async function report(n: number, today: string, owner: Owner): Promise<void> {
  // If a touched page couldn't be saved (conflict / disk error), DON'T reload the
  // journals feed — that would re-read the old files and drop the carried blocks
  // from memory. Leave the move in memory and surface the failure.
  if (!(await flushPage(today)) || !owner()) {
    if (!owner()) return;
    pushToast("Carry couldn't be saved — resolve the conflict; your moved tasks are kept in the editor.", "error");
    return;
  }
  // TODO(S2): explicit pane handle for the journals feed pane.
  openJournals({ inPlace: true }); // a carry reloads the feed in place, not a new tab
  pushToast(n ? `Carried ${n} item${n === 1 ? "" : "s"} to today` : "No unfinished tasks to carry");
}

/** Carry unfinished tasks from the previous *non-empty* day to today. "Previous
 *  day" means the most recent journal before today that actually has content
 *  (not literally yesterday, which is often blank). */
export async function carryPrevDay(): Promise<void> {
  const owner = graphOwner();
  const today = new Date();
  const todayKey =
    today.getFullYear() * 10000 + (today.getMonth() + 1) * 100 + today.getDate();
  let days: number[] = [];
  try {
    const result = await readOwned(owner, backend().journalContentDays());
    if (result.kind === "stale") return;
    days = result.value;
  } catch {
    days = [];
  }
  if (!owner()) return;
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
  const owner = graphOwner();
  const today = await ensureToday(owner);
  if (!today || !owner()) return;
  if (pageName === today) return;
  if (!(await ensureLoaded(pageName, "journal", owner))) return;
  if (!owner()) return;
  if (refuseConflictedMove([today, pageName])) return;
  const n = carryUnfinished([pageName], carryKeepsContext(), carryHeaderText());
  await report(n, today, owner);
}

/** Carry unfinished tasks from the last `days` days (today−1 … today−days) to
 *  today, newest first. Only days that have a file are touched. */
export async function carryDaysBack(days: number): Promise<void> {
  const owner = graphOwner();
  const today = await ensureToday(owner);
  if (!today || !owner()) return;
  const base = new Date();
  const candidates: string[] = [];
  for (let i = 1; i <= days; i++) {
    const d = new Date(base);
    d.setDate(d.getDate() - i);
    candidates.push(journalTitle(d));
  }
  // Load all the day files in parallel rather than one IPC round-trip at a time.
  const loaded = await Promise.all(candidates.map((t) => ensureLoaded(t, "journal", owner)));
  if (!owner()) return;
  const titles = candidates.filter((_, i) => loaded[i]); // skip days with no file
  if (refuseConflictedMove([today, ...titles])) return;
  const n = carryUnfinished(titles, carryKeepsContext(), carryHeaderText());
  await report(n, today, owner);
}
