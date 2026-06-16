// Journal page title in the default Logseq "MMM do, yyyy" format (matches the
// Rust JournalDate::title), so [[Today]]-style links and calendar navigation
// resolve to the right journal page.

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function ordinal(n: number): string {
  const a = n % 10;
  const b = n % 100;
  const suffix =
    (a === 1 && b !== 11) ? "st" : (a === 2 && b !== 12) ? "nd" : (a === 3 && b !== 13) ? "rd" : "th";
  return `${n}${suffix}`;
}

export function journalTitle(d: Date): string {
  return `${MONTHS[d.getMonth()]} ${ordinal(d.getDate())}, ${d.getFullYear()}`;
}
