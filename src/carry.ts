// "Carry unfinished tasks to today" (feature B). The store engine
// (carryUnfinished) does the tree surgery; this orchestrates loading the days it
// needs into the working set, then surfaces the result. Days are passed
// newest→oldest so the newest carried tasks end up on top of today.

import { backend } from "./backend";
import { readOwned, type Owner, bindingOwner } from "./owned";
import { pageByName, admitPageFile, ensurePageLoaded, carryUnfinished, flushPage, carryTodayPage, refuseConflictedMove, reportPageLoadRefusal } from "./document";
import { journalTitle, appNow } from "./journal";
import { carryKeepsContext, carryHeaderText } from "./ui";
import { pushToast } from "./toasts";
import { openJournals } from "./router";

async function ensureLoaded(name: string, kind: "journal" | "page", owner: Owner): Promise<boolean> {
  if (pageByName(name)) return true;
  const result = await readOwned(owner, backend().getPage(name, kind));
  if (result.kind === "stale" || !result.value) return false;
  // A declined replacement is a refusal: stop, never assume it loaded.
  const refusal = ensurePageLoaded(result.value);
  if (refusal) reportPageLoadRefusal(refusal, "Nothing was carried.");
  return !refusal && !!pageByName(name);
}

/** Make sure today's own file (or, with no file yet, an empty page) holds
 *  today's name. A second file for the same day — a duplicate day left by sync
 *  delivery or a journal date-format change, opened path-pinned — can hold the
 *  name; carrying into it would land the tasks in a file the journals feed does
 *  not show for today. Without unsaved input it is replaced by today's file;
 *  with unsaved input carry refuses, naming both files (og I1e, og J1; GH #254
 *  family, master 7bd793bd0; `admitPageFile`). One page read. */
async function ensureToday(owner: Owner): Promise<string | null> {
  const t = journalTitle(appNow());
  const admitted = await admitPageFile(t, "journal", owner, carryTodayPage(t));
  if (admitted === "stale") return null;
  if (admitted) {
    reportPageLoadRefusal(admitted, "Nothing was carried.");
    return null;
  }
  return pageByName(t) ? t : null;
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

/** Carry unfinished tasks from the latest earlier journal with content to
 * today. This scans and sorts the journal-day inventory; a lookup failure
 * reports an error toast. Moving is in memory before today's page
 * save; a save failure leaves moved tasks in the editor for resolution. */
export async function carryPrevDay(): Promise<void> {
  const owner = bindingOwner();
  const today = appNow();
  const todayKey =
    today.getFullYear() * 10000 + (today.getMonth() + 1) * 100 + today.getDate();
  let days: number[] = [];
  try {
    const result = await readOwned(owner, backend().journalContentDays());
    if (result.kind === "stale") return;
    days = result.value;
  } catch (error) {
    if (owner()) pushToast(`Could not read journal days for carry: ${String(error)}`, "error");
    return;
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

/** Carry unfinished tasks from a named journal to today. A missing source or
 * today's own page does nothing. The move changes the in-memory working set
 * and saves today's page; failed saves retain the moved tasks in the editor and
 * toast. Page-read failures also toast. Cost follows the source/day blocks
 * plus any page load and grouped save. */
export async function carryDay(pageName: string): Promise<void> {
  const owner = bindingOwner();
  try {
    const today = await ensureToday(owner);
    if (!today || !owner()) return;
    if (pageName === today) return;
    if (!(await ensureLoaded(pageName, "journal", owner))) return;
    if (!owner()) return;
    if (refuseConflictedMove([today, pageName])) return;
    const n = carryUnfinished([pageName], carryKeepsContext(), carryHeaderText());
    await report(n, today, owner);
  } catch (error) {
    if (owner()) pushToast(`Could not carry tasks: ${String(error)}`, "error");
  }
}

/** Carry unfinished tasks from today-1 through today-days, newest first,
 * skipping missing files. Starts one page lookup per requested day in parallel;
 * work grows with days and the loaded blocks. The numeric argument is not
 * clamped or validated. A failed final save leaves moves in memory and toasts. */
export async function carryDaysBack(days: number): Promise<void> {
  const owner = bindingOwner();
  try {
    const today = await ensureToday(owner);
    if (!today || !owner()) return;
    const base = appNow();
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
  } catch (error) {
    if (owner()) pushToast(`Could not carry tasks: ${String(error)}`, "error");
  }
}
