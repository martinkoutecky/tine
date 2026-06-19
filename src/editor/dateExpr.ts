// Frontend resolver for the `between` date tokens the Rust engine understands
// (query.rs::resolve_date_token / parse_relative). Used by the query builder to
// preview a typed bound as a concrete date and to offer one-click relative
// presets. Kept dependency-free and on the frontend so the date picker never
// blocks on IPC. Journal-page-title tokens are NOT resolved here (that needs the
// graph); they pass through to the backend verbatim.

import { journalTitle } from "../journal";

const MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

function addDays(d: Date, n: number): Date {
  const r = new Date(d);
  r.setDate(r.getDate() + n);
  return r;
}
// Build a Date only if the y/m/d round-trip exactly (rejects 2026-02-31 etc.).
function validDate(y: number, m: number, d: number): Date | null {
  const dt = new Date(y, m, d);
  return dt.getFullYear() === y && dt.getMonth() === m && dt.getDate() === d ? dt : null;
}
function addMonths(d: Date, n: number): Date {
  const r = new Date(d);
  const day = r.getDate();
  r.setDate(1);
  r.setMonth(r.getMonth() + n);
  // Clamp to the last valid day of the target month (mirrors add_months).
  const last = new Date(r.getFullYear(), r.getMonth() + 1, 0).getDate();
  r.setDate(Math.min(day, last));
  return r;
}

/** Resolve a bound token to a concrete date, or null if it needs the graph
 *  (a journal page title) or is malformed. `today` defaults to the real today. */
export function resolveDateToken(tok: string, today = new Date()): Date | null {
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
  // Signed relative duration: ±N[dwmy].
  const rel = /^([+-]?)(\d+)([dwmy])$/i.exec(t);
  if (rel) {
    const n = (rel[1] === "-" ? -1 : 1) * parseInt(rel[2], 10);
    switch (rel[3].toLowerCase()) {
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
  // ISO yyyy-MM-dd. Reject impossible dates (e.g. 2026-02-31) rather than letting
  // JS Date silently roll them over to a wrong day.
  const iso = /^(\d{4})-(\d{2})-(\d{2})$/.exec(t);
  if (iso) {
    return validDate(+iso[1], +iso[2] - 1, +iso[3]);
  }
  // "MMM do, yyyy" journal title (e.g. "Jun 16th, 2026") — resolvable locally.
  const jt = /^([a-z]{3})\s+(\d{1,2})(?:st|nd|rd|th)?,?\s+(\d{4})$/i.exec(t);
  if (jt) {
    const m = MONTHS.indexOf(jt[1].toLowerCase());
    if (m >= 0) return validDate(+jt[3], m, +jt[2]);
  }
  return null;
}

/** Short, human preview of a resolved token ("→ Jun 16th, 2026"), or "" if the
 *  token can't be resolved on the frontend. */
export function previewDate(tok: string, today = new Date()): string {
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
