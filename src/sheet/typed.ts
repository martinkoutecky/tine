const PLAIN_DECIMAL_RE = /^[+-]?\d+(?:\.\d+)?$/;

export function isPlainDecimalNumber(value: string): boolean {
  return PLAIN_DECIMAL_RE.test(value);
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
  if (mo < 1 || mo > 12 || hh > 23 || mm > 59) return null;
  const dt = new Date(Date.UTC(y, mo - 1, d, hh, mm));
  if (dt.getUTCFullYear() !== y || dt.getUTCMonth() !== mo - 1 || dt.getUTCDate() !== d) return null;
  return { y, m: mo - 1, d, time: m[4] == null ? null : `${m[4]}:${m[5]}` };
}

/** Loose prefix form for aggregating over mixed cell text: a leading
 *  `yyyy-mm-dd`, optionally wrapped as OG planning `<yyyy-mm-dd …>`. */
export function isoDatePrefix(text: string): string | null {
  const m = /^<?(\d{4}-\d{2}-\d{2})/.exec(text);
  return m ? m[1] : null;
}
