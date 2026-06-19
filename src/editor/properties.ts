// Pure helpers for reading/editing `key:: value` property lines — a block's
// continuation lines or a page's pre-block. No store/DOM, so unit-testable.

export const PROP_LINE = /^([A-Za-z0-9_./-]+):: ?(.*)$/;

// Built-in properties hidden from the editor by default (like OG): `id::` and
// `collapsed::` are kept in the file for persistence but never shown in the edit
// textarea. Annotation (PDF highlight) blocks instead hide ALL properties and
// edit only their text.
const BUILTIN_HIDDEN = new Set(["id", "collapsed"]);
/** Hide just the built-in `id::`/`collapsed::` properties (normal blocks). */
export const isBuiltinHidden = (key: string): boolean => BUILTIN_HIDDEN.has(key);
/** Hide every property (annotation blocks edit only their text). */
export const hideAll = (_key: string): boolean => true;

function propLineKey(line: string): string | null {
  const m = /^\s*([A-Za-z0-9_./-]+)::/.exec(line);
  return m ? m[1].toLowerCase() : null;
}

/** Split a block's raw into the editor-visible text and the hidden property
 *  lines. Fence-aware: a `key:: value` line inside a ```/~~~ code fence stays
 *  visible content — it must NOT be pulled out as metadata and reattached
 *  outside the fence (which would corrupt the code on focus+blur). `isHidden`
 *  selects which property keys are hidden (e.g. {@link isBuiltinHidden} or
 *  {@link hideAll}). Inverse of {@link joinProps}. */
export function splitProps(
  raw: string,
  isHidden: (key: string) => boolean
): { visible: string; hidden: string } {
  const vis: string[] = [];
  const hid: string[] = [];
  let fence: string | null = null;
  for (const l of raw.split("\n")) {
    const fm = /^\s*(`{3,}|~{3,})/.exec(l);
    if (fm) {
      const ch = fm[1][0];
      if (fence === null) fence = ch;
      else if (ch === fence) fence = null;
      vis.push(l);
      continue;
    }
    if (fence !== null) {
      vis.push(l); // inside a code fence — never metadata
      continue;
    }
    const k = propLineKey(l);
    (k && isHidden(k) ? hid : vis).push(l);
  }
  return { visible: vis.join("\n"), hidden: hid.join("\n") };
}

/** Reattach hidden property lines below the visible text. */
export function joinProps(visible: string, hidden: string): string {
  return hidden ? `${visible}\n${hidden}` : visible;
}

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
