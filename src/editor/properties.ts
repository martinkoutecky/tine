// Pure helpers for reading/editing `key:: value` property lines — a block's
// continuation lines or a page's pre-block. No store/DOM, so unit-testable.

export const PROP_LINE = /^([A-Za-z0-9_./-]+):: ?(.*)$/;

/** First value for `key` (case-insensitive) in a property block, or null. */
export function readPropertyValue(block: string | null, key: string): string | null {
  if (!block) return null;
  for (const l of block.split("\n")) {
    const m = PROP_LINE.exec(l);
    if (m && m[1].toLowerCase() === key.toLowerCase()) return m[2].trim();
  }
  return null;
}

/** Add / replace / remove a `key:: value` line. A null or empty value removes
 *  the key. Other property lines are preserved (blank lines dropped); returns
 *  null when nothing is left (so an emptied pre-block isn't written as "").  */
export function upsertPropertyLine(
  block: string | null,
  key: string,
  value: string | null
): string | null {
  const kept = (block ?? "")
    .split("\n")
    .filter((l) => {
      const m = PROP_LINE.exec(l);
      return !(m && m[1].toLowerCase() === key.toLowerCase());
    })
    .filter((l) => l.trim() !== "");
  const v = value == null ? null : value.trim();
  if (v) kept.push(`${key}:: ${v}`);
  return kept.length ? kept.join("\n") : null;
}

/** The page-level properties we surface in the page-properties panel, with a
 *  one-line description and whether the value is a boolean toggle. */
export interface PagePropSpec {
  key: string;
  label: string;
  hint: string;
  kind: "text" | "bool" | "list";
}
export const PAGE_PROP_SPECS: PagePropSpec[] = [
  { key: "alias", label: "Aliases", hint: "Other names this page answers to in [[links]] (comma-separated)", kind: "list" },
  { key: "tags", label: "Tags", hint: "Page-level tags (comma-separated)", kind: "list" },
  { key: "title", label: "Display title", hint: "Override the shown title (the file name stays the same)", kind: "text" },
  { key: "icon", label: "Icon", hint: "An emoji/character shown with the title", kind: "text" },
  { key: "public", label: "Public", hint: "Include this page when exporting/publishing public pages", kind: "bool" },
];
