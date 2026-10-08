// Frontend resolver for the `between` date tokens the Rust engine understands
// (query.rs::resolve_date_token / parse_relative). Used by the query builder to
// preview a typed bound as a concrete date and to offer one-click relative
// presets. Kept dependency-free and on the frontend so the date picker never
// blocks on IPC. Journal-page-title tokens are NOT resolved here (that needs the
// graph); they pass through to the backend verbatim.
// The grammar is the engine's, not a second one: dateExpr.test.ts reads the same
// golden file as the Rust resolver (tests/fixtures/i12-date-token-golden.json).

import { journalTitle, appNow, localCalendarDate } from "../journal";

const MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

function addDays(d: Date, n: number): Date {
  const r = new Date(d);
  r.setDate(r.getDate() + n);
  return r;
}
function daysInMonth(year: number, month0: number): number {
  const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  return month0 === 1 ? (leap ? 29 : 28) : [3, 5, 8, 10].includes(month0) ? 30 : 31;
}
/** Mirrors `JournalDate::add_months`: carry into years, clamp the day to the
 *  target month. Pure arithmetic then one `setFullYear(y, m, d)`, so years 0-99
 *  stay literal (a `new Date(y, ...)` constructor would remap them to 19xx). */
function addMonths(d: Date, n: number): Date {
  const total = d.getMonth() + n;
  const year = d.getFullYear() + Math.floor(total / 12);
  const month = ((total % 12) + 12) % 12;
  const r = new Date(d);
  r.setFullYear(year, month, Math.min(d.getDate(), daysInMonth(year, month)));
  return r;
}

/** The largest relative offset, in years (the same span in days, weeks and
 *  months): `MAX_DATE_OFFSET_YEARS` in advanced_patterns.rs. */
const MAX_OFFSET_YEARS = 10_000;
const UNITS_PER_YEAR = { d: 366, w: 53, m: 12, y: 1 } as const;

/** Resolve a bound token to a concrete date, or null if it needs the graph
 *  (a journal page title) or is malformed. `today` defaults to the real today. */
export function resolveDateToken(tok: string, today = appNow()): Date | null {
  const t = tok.trim();
  switch (t.toLowerCase()) {
    case "":
      return null;
    case "today":
    case "now":
      return today;
    case "yesterday":
      return addDays(today, -1);
    case "tomorrow":
      return addDays(today, 1);
  }
  // Signed relative duration: [+-]N[dwmy], ASCII digits and a LOWERCASE unit
  // (`-7D` is not one), bounded like the engine's `DateToken::relative`.
  const rel = /^([+-]?)(\d+)([dwmy])$/.exec(t);
  if (rel) {
    const unit = rel[3] as keyof typeof UNITS_PER_YEAR;
    const magnitude = Number(rel[2]);
    if (!(magnitude <= MAX_OFFSET_YEARS * UNITS_PER_YEAR[unit])) return null;
    const n = (rel[1] === "-" ? -1 : 1) * magnitude;
    switch (unit) {
      case "d":
        return addDays(today, n);
      case "w":
        return addDays(today, n * 7);
      case "m":
        return addMonths(today, n);
      case "y":
        return addMonths(today, n * 12);
    }
  }
  // A `yyyy-MM-dd` or `yyyy_MM_dd` stem, zero padding optional
  // (`JournalDate::from_file_stem`). Impossible dates (2026-02-31) are rejected
  // rather than letting JS Date roll them over to a wrong day.
  const parts = t.split(t.includes("_") ? "_" : "-");
  if (parts.length === 3 && parts.every((part) => /^\d+$/.test(part))) {
    const [y, m, d] = parts.map(Number);
    if (y <= 2_147_483_647) return localCalendarDate(y, m - 1, d);
  }
  // "MMM do, yyyy" journal title (e.g. "Jun 16th, 2026"): the default title
  // format, one space after the month and after the comma, 1-4 year digits.
  const jt = /^([a-z]{3}) (\d{1,2})(?:st|nd|rd|th), (\d{1,4})$/i.exec(t);
  if (jt) {
    const m = MONTHS.indexOf(jt[1].toLowerCase());
    if (m >= 0) return localCalendarDate(+jt[3], m, +jt[2]);
  }
  return null;
}

/** Short, human preview of a resolved token ("→ Jun 16th, 2026"), or "" if the
 *  token can't be resolved on the frontend. */
export function previewDate(tok: string, today = appNow()): string {
  const d = resolveDateToken(tok, today);
  return d ? journalTitle(d) : "";
}

/** Relative-range presets for the builder: each yields [startToken, endToken]
 *  using the same DSL tokens the engine resolves. */
export interface DatePreset {
  label: string;
  start: string;
  end: string;
}
export const DATE_PRESETS: DatePreset[] = [
  { label: "Today", start: "today", end: "today" },
  { label: "Last 7 days", start: "-7d", end: "today" },
  { label: "Last 30 days", start: "-30d", end: "today" },
  { label: "Next 7 days", start: "today", end: "+7d" },
  { label: "This week", start: "-1w", end: "+1w" },
];
