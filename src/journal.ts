import { createSignal } from "solid-js";
import { format_journal_date, parse_journal_format_json } from "./render/wasm/lsdoc_wasm.js";

/** Journal dates and title formatting. `journalTitle` and `parseJournalTitle`
 * read the graph's active display format set by `setJournalTitleFormat`;
 * `formatJournal` and `parseJournalWith` take an explicit format. All operations
 * are independent of graph size and write no files. Parsing returns null for
 * unrecognized/invalid titles; a configured format defaults to "MMM do, yyyy". */

const DEFAULT_TITLE_FORMAT = "MMM do, yyyy";
let titleFormat = DEFAULT_TITLE_FORMAT;

/** The app's wall clock (GH #607). The backend's time-zone rules are the
 * calendar authority: journal membership, the feed's `as_of_day` and relative
 * queries all use them. A WebView can carry older zone rules than the OS (the
 * AppImage bundles its own ICU, which still applied Mexico City's abolished
 * daylight saving time), and a frontend "today" read from `new Date()` then
 * disagreed with the backend's for an hour a night, stalling the journal feed.
 * `appNow()` is `new Date()` shifted by the difference between the two zone
 * offsets at one instant: zero whenever both sides agree, so it changes nothing
 * for a WebView with current rules. Its local getters read the backend's wall
 * clock. Every frontend read of "now" goes through here
 * (`src/appClock.guard.test.ts`). O(1), no I/O. */
type BackendClock = { offset_minutes: number; unix_ms: number };
let zoneSkewMs = 0;

/** Stale zone rules move an offset by an hour or two (a DST rule, a zone
 *  re-basing); a wider disagreement is likelier a backend that failed to detect
 *  the zone at all (and fell back to UTC), where the WebView is the better
 *  witness. */
const MAX_ZONE_SKEW_MINUTES = 180;

function skewFor(clock: BackendClock): number {
  const browserOffset = -new Date(clock.unix_ms).getTimezoneOffset();
  const skew = clock.offset_minutes - browserOffset;
  return Math.abs(skew) > MAX_ZONE_SKEW_MINUTES ? 0 : skew * 60_000;
}

{
  const injected = (globalThis as { __TINE_LOCAL_CLOCK__?: BackendClock }).__TINE_LOCAL_CLOCK__;
  if (injected && Number.isFinite(injected.offset_minutes) && Number.isFinite(injected.unix_ms)) {
    zoneSkewMs = skewFor(injected);
  }
}

export function appNow(): Date {
  return new Date(Date.now() + zoneSkewMs);
}

/** Adopt a fresh backend clock sample; re-derives the reactive day key when the
 *  correction changes. */
export function setBackendClock(clock: BackendClock): void {
  if (!Number.isFinite(clock.offset_minutes) || !Number.isFinite(clock.unix_ms)) return;
  const skew = skewFor(clock);
  if (skew === zoneSkewMs) return;
  zoneSkewMs = skew;
  setDayKey(localDayKey());
}

let clockSourceInstalled = false;
/** Keep the correction current: now, on focus/visibility, and every ten minutes
 *  (a zone transition moves both offsets at once for an agreeing WebView, so a
 *  stale sample can only err where the correction was already needed). */
export function installBackendClock(read: () => Promise<BackendClock>): void {
  if (clockSourceInstalled || typeof window === "undefined") return;
  clockSourceInstalled = true;
  const refresh = () => void read().then(setBackendClock, () => {});
  refresh();
  window.setInterval(refresh, 10 * 60_000);
  window.addEventListener("focus", refresh);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) refresh();
  });
}

/** Stable local calendar day, also across DST changes. */
export function localDayKey(now = appNow()): number {
  return now.getFullYear() * 10_000 + (now.getMonth() + 1) * 100 + now.getDate();
}

/** Inverse for a valid local yyyymmdd key, returning local midnight. Invalid
 * keys normalize as JS dates do; nonfinite keys produce an invalid Date. */
export function localDateFromDayKey(key: number): Date {
  const date = new Date(0);
  date.setHours(0, 0, 0, 0);
  date.setFullYear(Math.floor(key / 10_000), Math.floor((key % 10_000) / 100) - 1, key % 100);
  return date;
}

/** Milliseconds until the next local midnight plus margin, clamped to at least 1. */
export function localDayRolloverDelay(now = appNow(), marginMs = 25): number {
  const next = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  return Math.max(1, next.getTime() - now.getTime() + marginMs);
}

const [dayKey, setDayKey] = createSignal(localDayKey());
let dayKeyArmed = false;

/** Test-only override of the reactive day signal until the next clock sync or
 * rollover timer; the real clock is unchanged. */
export function setCurrentDayKeyForTest(key: number): void { setDayKey(key); }

/** Reactive local day for controls that stay mounted through midnight. O(1),
 * without file I/O. The first browser call synchronizes from the clock and
 * installs one recurring midnight timer plus focus/visibility listeners for
 * the module lifetime; later calls read the signal. There is no disposer. */
export function currentDayKey(): number {
  if (!dayKeyArmed && typeof window !== "undefined") {
    dayKeyArmed = true;
    setDayKey(localDayKey());
    const arm = () => {
      window.setTimeout(() => { setDayKey(localDayKey()); arm(); }, localDayRolloverDelay());
    };
    arm();
    const sync = () => setDayKey(localDayKey());
    window.addEventListener("focus", sync);
    document.addEventListener("visibilitychange", () => { if (!document.hidden) sync(); });
  }
  return dayKey();
}

export type JournalDateParts = { y: number; m: number; d: number };

/// Set the active journal title format (from `GraphMeta.journal_page_title_format`).
export function setJournalTitleFormat(fmt: string | undefined | null): void {
  titleFormat = fmt && fmt.trim() ? fmt : DEFAULT_TITLE_FORMAT;
}

/** Native format grammar with a bounded compiled-pattern cache. O(format + title), no I/O. */
export function formatJournal(d: Date, fmt: string): string {
  return format_journal_date(d.getFullYear(), d.getMonth() + 1, d.getDate(), fmt);
}

/** Construct local calendar parts without Date's 1900 offset for years 0–99.
 * Reject impossible dates. O(1), no I/O; month is zero-based. */
export function localCalendarDate(y: number, m: number, d: number): Date | null {
  const date = new Date(2000, 0, 1);
  date.setFullYear(y, m, d);
  return date.getFullYear() === y && date.getMonth() === m && date.getDate() === d ? date : null;
}

/// The journal page title for a date, in the graph's configured title format.
export function journalTitle(d: Date): string {
  return formatJournal(d, titleFormat);
}

/// Try to parse `s` as a date in pattern `fmt` (the inverse of formatJournal, for
/// the token subset Logseq uses). Mirrors the Rust `Format::parse` so a
/// `[[journal title]]` link can be routed to the journal page rather than opened
/// as an empty regular page. Returns the date iff the whole string is valid.
const parsedTitles = new Map<string, JournalDateParts | null>();
export function parseJournalWith(s: string, fmt: string): JournalDateParts | null {
  const key = JSON.stringify([fmt,s]);
  if (parsedTitles.has(key)) return parsedTitles.get(key)!;
  const parts = JSON.parse(parse_journal_format_json(s, fmt)) as JournalDateParts | null;
  if (parsedTitles.size === 64) parsedTitles.delete(parsedTitles.keys().next().value!);
  parsedTitles.set(key,parts);
  return parts;
}

/// Whether `name` is a journal date in the graph's title format (or a common
/// default) — so a `[[name]]` link / quick-switch pick opens the journal, not an
/// empty page. Mirrors the backend's `safe-journal-title-formatters` leniency.
export function isJournalTitle(name: string): boolean {
  return journalParts(name) !== null;
}

/** Parse a journal title as a local Date from the active format, default title
 * format, ISO or underscore date. Returns null on invalid/unrecognized input.
 * O(title length), independent of graph size; never reads a page. */
function journalParts(name: string): JournalDateParts | null {
  for (const fmt of [titleFormat, DEFAULT_TITLE_FORMAT, "yyyy-MM-dd", "yyyy_MM_dd"]) {
    const parts = parseJournalWith(name.trim(), fmt);
    if (parts) return parts;
  }
  return null;
}

export function parseJournalTitle(name: string): Date | null {
  const parts = journalParts(name);
  return parts ? localCalendarDate(parts.y, parts.m - 1, parts.d) : null;
}
