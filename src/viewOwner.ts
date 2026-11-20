import { loadedPage } from "./document/model";
import { ownerPath, resolvedTarget } from "./pageIndex";
import type { PageKind } from "./types";
import type { SidebarItem } from "./ui";

/** The one answer to "which file does this view show", for sidebar items and
 *  tab routes alike: its own exact path; else, for a name-only view, the file
 *  loaded under that name (what it displays, also while the inventory is
 *  refreshing); else the file the store resolves the name to. */
export function viewOwnerPath(name: string, pageKind: PageKind, path: string | undefined): string | undefined {
  if (path) return path;
  const loaded = loadedPage(name);
  if (loaded?.id && loaded.kind === pageKind) return loaded.id;
  return ownerPath(resolvedTarget(name, pageKind));
}

/** Same-owner check of both sidebar open handlers: a pathful result never
 *  evicts the name-only view of the same file, and a different same-name file
 *  still replaces it. */
export function sameSidebarOwner(item: SidebarItem, name: string, pageKind: PageKind, path: string | undefined): boolean {
  const itemName = item.kind === "page" ? item.name : item.page;
  if (itemName !== name || item.pageKind !== pageKind) return false;
  return viewOwnerPath(itemName, item.pageKind, item.path) === viewOwnerPath(name, pageKind, path);
}
