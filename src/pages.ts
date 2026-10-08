import type { PageEntry } from "./types";

// All Pages and the namespace name list are views of the one page index
// (`pageIndex.ts`); this module keeps only the list labels.
export { allPages, allPageNames } from "./pageIndex";

function parentPathLabel(p: PageEntry): string {
  const root = p.kind === "journal" ? "journals/" : "pages/";
  const rel = p.path.startsWith(root) ? p.path.slice(root.length) : p.path;
  const slash = rel.lastIndexOf("/");
  return slash >= 0 ? `${rel.slice(0, slash)}/` : root;
}

/** Disambiguating labels for a whole page list, in ONE pass over it: O(pages) to
 *  build, O(1) per row. Two pages can share a display name (same title in two
 *  folders, or one stem as `.md` and `.org`); such a row grows its parent
 *  sub-path, and its full path if that is still ambiguous. Deciding that per
 *  row scans the whole list per row (O(rows x pages), master F9). Keyed by
 *  VALUE, so a caller holding a copy of an entry gets the same label. */
export function pageListLabels(pages: PageEntry[]): (p: PageEntry) => string {
  const nameKey = (p: PageEntry) => `${p.kind}\0${p.name}`;
  const parentKey = (p: PageEntry) => `${nameKey(p)}\0${parentPathLabel(p)}`;
  const byName = new Map<string, number>();
  const byNameAndParent = new Map<string, number>();
  for (const p of pages) {
    byName.set(nameKey(p), (byName.get(nameKey(p)) ?? 0) + 1);
    byNameAndParent.set(parentKey(p), (byNameAndParent.get(parentKey(p)) ?? 0) + 1);
  }
  return (p) => {
    if ((byName.get(nameKey(p)) ?? 0) < 2) return p.name;
    const unique = (byNameAndParent.get(parentKey(p)) ?? 0) === 1;
    return `${p.name} — ${unique ? parentPathLabel(p) : p.path}`;
  };
}
