const PLAIN_DECIMAL_RE = /^[+-]?\d+(?:\.\d+)?$/;

export function isPlainDecimalNumber(value: string): boolean {
  return PLAIN_DECIMAL_RE.test(value);
}

/** Finite numeric cell value, O(value bytes), without allocations. Decimal
 * cells require the whole token; stable's aggregate-prefix policy also accepts
 * units, exponents and leading numbers in mixed text. Neither parses queries. */
export function sheetNumber(value: string, policy: "decimal" | "aggregate-prefix"): number | null {
  const n = policy === "aggregate-prefix" ? parseFloat(value)
    : isPlainDecimalNumber(value) ? Number(value) : NaN;
  return Number.isFinite(n) ? n : null;
}

const DAY_MS = 86_400_000;
/** UTC calendar construction with an explicit full year, O(1), no allocation.
 * Date.UTC treats 0–99 as 1900–1999. A Gregorian 400-year cycle avoids that
 * remapping while preserving leap days. Overflow follows JS calendar arithmetic. */
export function utcCalendarMillis(y: number, m: number, d: number, hour = 0, minute = 0): number {
  return y >= 0 && y < 100
    ? Date.UTC(y + 400, m, d, hour, minute) - 146_097 * DAY_MS
    : Date.UTC(y, m, d, hour, minute);
}

export function daysInCalendarMonth(y: number, m: number): number {
  return (utcCalendarMillis(y, m + 1, 1) - utcCalendarMillis(y, m, 1)) / DAY_MS;
}

function validIsoDay(y: number, m: number, d: number): boolean {
  return m >= 1 && m <= 12 && d >= 1 && d <= daysInCalendarMonth(y, m - 1);
}

// The ONE recognizer of the sheet ISO date grammar (`yyyy-mm-dd`, optional
// ` HH:MM`/`THH:MM` tail) — DatePicker, typed cells, and field writes all
// read through here; a second regex of this shape elsewhere is a bug.
export interface IsoDateParts {
  y: number;
  m: number; // 0-based month, Date-style
  d: number;
  time: string | null;
}

export function parseIsoDateLike(value: string): IsoDateParts | null {
  const m = /^\s*(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2}))?\s*$/.exec(value);
  if (!m) return null;
  const y = Number(m[1]);
  const mo = Number(m[2]);
  const d = Number(m[3]);
  const hh = m[4] == null ? 0 : Number(m[4]);
  const mm = m[5] == null ? 0 : Number(m[5]);
  if (!validIsoDay(y, mo, d) || hh > 23 || mm > 59) return null;
  return { y, m: mo - 1, d, time: m[4] == null ? null : `${m[4]}:${m[5]}` };
}

/** Aggregate-prefix policy: a calendar-valid leading `yyyy-mm-dd`, optionally
 * wrapped as OG planning `<yyyy-mm-dd …>`. Trailing text is deliberately allowed;
 * typed date cells instead require the entire date/time token. O(prefix bytes). */
export function isoDatePrefix(text: string): string | null {
  const m = /^<?(\d{4}-\d{2}-\d{2})/.exec(text);
  if (!m) return null;
  const iso = m[1];
  return validIsoDay(Number(iso.slice(0, 4)), Number(iso.slice(5, 7)), Number(iso.slice(8, 10))) ? iso : null;
}
