import { pageIdentityKey } from "./pageIdentity";
import type { PageInventoryEntry } from "./types";

/** The tree parent candidate the store answers for an ordinary page name
 *  (`InventoryEntry::parent`): the page its immediate nonempty prefix names,
 *  for these test names (no boundary slash, no date prefix). A name that is
 *  no namespace name (`./x`, `../x`, a URL with an origin) has none
 *  (`refs::namespace_refs`). */
export function ordinaryParent(name: string, notNamespace = false): PageInventoryEntry["parent"] {
  if (notNamespace || name.startsWith("./") || name.startsWith("../") || /^[a-z][a-z0-9+.-]*:\/\//i.test(name)) return undefined;
  const segs = name.split("/");
  let depth = segs.length - 1;
  while (depth > 0 && !segs.slice(0, depth).join("/")) depth--;
  return depth > 0 ? { name: segs.slice(0, depth).join("/"), kind: "page" } : undefined;
}

/** `row` as the store lists it: an ordinary page row carries its tree parent.
 *  The namespace views derive no parent themselves, so a test that hands them
 *  rows without one installs these. */
export function withOrdinaryParent<T extends PageInventoryEntry>(row: T): T {
  if (row.parent || row.is_journal) return row;
  const parent = ordinaryParent(row.name, row.not_namespace === true);
  return parent ? { ...row, parent } : row;
}

/** Inventory rows for ordinary page `names`, each with its tree parent. */
export function ordinaryRows(names: readonly string[]): PageInventoryEntry[] {
  return names.map((name) =>
    withOrdinaryParent({
      key: pageIdentityKey(name),
      name,
      is_journal: false,
      day: null,
      target: { kind: "absent", id: `pages/${name}.md` },
    }),
  );
}
