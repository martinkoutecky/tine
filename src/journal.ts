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
