// Journal page title formatting. Mirrors the Rust `date::Format` so a title the
// frontend computes for "today" matches the one the backend derives from the
// journal file — including a graph's custom `:journal/page-title-format`. The
// current format is fed in from GraphMeta on graph load (see graph.ts);
// it defaults to Logseq's "MMM do, yyyy".

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_FULL = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];
// Indexed by Date.getDay() (0 = Sunday), matching the Rust formatter.
const WD_FULL = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const WD_ABBR = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WD_2 = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
const WD_1 = ["S", "M", "T", "W", "T", "F", "S"];

const DEFAULT_TITLE_FORMAT = "MMM do, yyyy";
let titleFormat = DEFAULT_TITLE_FORMAT;

export type JournalDateParts = { y: number; m: number; d: number };

/// Set the active journal title format (from `GraphMeta.journal_page_title_format`).
export function setJournalTitleFormat(fmt: string | undefined | null): void {
  titleFormat = fmt && fmt.trim() ? fmt : DEFAULT_TITLE_FORMAT;
}

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

function ordinal(n: number): string {
  const a = n % 10;
  const b = n % 100;
  const suffix =
    a === 1 && b !== 11 ? "st" : a === 2 && b !== 12 ? "nd" : a === 3 && b !== 13 ? "rd" : "th";
  return `${n}${suffix}`;
}

/// Format a date with an explicit cljs-time/Joda-style pattern (the subset Logseq
/// uses). Unknown characters are emitted literally.
export function formatJournal(d: Date, fmt: string): string {
  const y = d.getFullYear();
  const mo = d.getMonth(); // 0-based
  const day = d.getDate();
  const dow = d.getDay(); // 0 = Sunday
  let out = "";
  let i = 0;
  const at = (s: string) => fmt.startsWith(s, i);
  while (i < fmt.length) {
    if (at("yyyy")) { out += String(y).padStart(4, "0"); i += 4; }
    else if (at("yy")) { out += pad2(((y % 100) + 100) % 100); i += 2; }
    else if (at("y")) { out += String(y); i += 1; }
    else if (at("MMMM")) { out += MONTHS_FULL[mo]; i += 4; }
    else if (at("MMM")) { out += MONTHS[mo]; i += 3; }
    else if (at("MM")) { out += pad2(mo + 1); i += 2; }
    else if (at("M")) { out += String(mo + 1); i += 1; }
    else if (at("dd")) { out += pad2(day); i += 2; }
    else if (at("do")) { out += ordinal(day); i += 2; }
    else if (at("d")) { out += String(day); i += 1; }
    else if (at("EEEE")) { out += WD_FULL[dow]; i += 4; }
    else if (at("EEE")) { out += WD_ABBR[dow]; i += 3; }
    else if (at("EE")) { out += WD_2[dow]; i += 2; }
    else if (at("E")) { out += WD_1[dow]; i += 1; }
    else { out += fmt[i]; i += 1; }
  }
  return out;
}

/// The journal page title for a date, in the graph's configured title format.
export function journalTitle(d: Date): string {
  return formatJournal(d, titleFormat);
}

/// Try to parse `s` as a date in pattern `fmt` (the inverse of formatJournal, for
/// the token subset Logseq uses). Mirrors the Rust `Format::parse` so a
/// `[[journal title]]` link can be routed to the journal page rather than opened
/// as an empty regular page. Returns the date iff the whole string is valid.
export function parseJournalWith(s: string, fmt: string): JournalDateParts | null {
  let i = 0;
  let f = 0;
  let y: number | null = null;
  let mo: number | null = null;
  let d: number | null = null;
  const at = (t: string) => fmt.startsWith(t, f);
  const digits = (max: number): number | null => {
    const st = i;
    while (i < s.length && i - st < max && s[i] >= "0" && s[i] <= "9") i++;
    return i > st ? parseInt(s.slice(st, i), 10) : null;
  };
  const matchName = (tables: string[][]): number | null => {
    for (const tbl of tables) {
      for (let idx = 0; idx < tbl.length; idx++) {
        const nm = tbl[idx];
        if (nm && s.substr(i, nm.length).toLowerCase() === nm.toLowerCase()) {
          i += nm.length;
          return idx;
        }
      }
    }
    return null;
  };
  while (f < fmt.length) {
    if (at("yyyy")) { const v = digits(4); if (v === null) return null; y = v; f += 4; }
    else if (at("yy")) { const v = digits(2); if (v === null) return null; y = 2000 + v; f += 2; }
    else if (at("y")) { const v = digits(4); if (v === null) return null; y = v; f += 1; }
    else if (at("MMMM") || at("MMM")) {
      const idx = matchName([MONTHS_FULL, MONTHS]);
      if (idx === null) return null;
      mo = idx + 1;
      f += at("MMMM") ? 4 : 3;
    } else if (at("MM")) { const v = digits(2); if (v === null) return null; mo = v; f += 2; }
    else if (at("M")) { const v = digits(2); if (v === null) return null; mo = v; f += 1; }
    else if (at("dd")) { const v = digits(2); if (v === null) return null; d = v; f += 2; }
    else if (at("do")) {
      const v = digits(2);
      if (v === null) return null;
      d = v;
      f += 2;
      // OG cljs-time's parse-ordinal-suffix accepts any suffix string here; it
      // does not validate that "st"/"nd"/"rd"/"th" matches the day number.
      if (!["st", "nd", "rd", "th"].includes(s.substr(i, 2).toLowerCase())) return null;
      i += 2;
    } else if (at("d")) { const v = digits(2); if (v === null) return null; d = v; f += 1; }
    else if (at("EEEE") || at("EEE") || at("EE") || at("E")) {
      const tok = at("EEEE") ? 4 : at("EEE") ? 3 : at("EE") ? 2 : 1;
      if (matchName([WD_FULL, WD_ABBR, WD_2, WD_1]) === null) return null;
      f += tok;
    } else {
      if (s[i] !== fmt[f]) return null;
      i += 1;
      f += 1;
    }
  }
  if (
    i !== s.length ||
    y === null ||
    mo === null ||
    d === null ||
    mo < 1 ||
    mo > 12 ||
    d < 1 ||
    d > 31
  ) {
    return null;
  }
  return { y, m: mo, d };
}

/// Whether `name` is a journal date in the graph's title format (or a common
/// default) — so a `[[name]]` link / quick-switch pick opens the journal, not an
/// empty page. Mirrors the backend's `safe-journal-title-formatters` leniency.
export function isJournalTitle(name: string): boolean {
  if (!name.trim()) return false;
  return [titleFormat, DEFAULT_TITLE_FORMAT, "yyyy-MM-dd"].some((f) => parseJournalWith(name, f) !== null);
}
