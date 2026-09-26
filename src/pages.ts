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

export function pageListLabel(p: PageEntry, pages: PageEntry[]): string {
  const same = pages.filter((x) => x.kind === p.kind && x.name === p.name);
  if (same.length < 2) return p.name;
  const label = parentPathLabel(p);
  const unique = same.filter((x) => parentPathLabel(x) === label).length === 1;
  return `${p.name} — ${unique ? label : p.path}`;
}
