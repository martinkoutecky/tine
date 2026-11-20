import { ownerPath, resolvedTarget } from "./pageIndex";
import type { PageKind } from "./types";
import type { SidebarItem } from "./ui";

/** The file a sidebar item shows: its own path, else the file the store
 *  resolves its name to. The one comparison for "same owner" in both sidebar
 *  open handlers, so a pathful result never evicts the name-only view of the
 *  same file, and a different same-name file still replaces it. */
function sidebarOwnerPath(name: string, pageKind: PageKind, path: string | undefined): string | undefined {
  return path ?? ownerPath(resolvedTarget(name, pageKind));
}
export function sameSidebarOwner(item: SidebarItem, name: string, pageKind: PageKind, path: string | undefined): boolean {
  const itemName = item.kind === "page" ? item.name : item.page;
  if (itemName !== name || item.pageKind !== pageKind) return false;
  return sidebarOwnerPath(itemName, item.pageKind, item.path) === sidebarOwnerPath(name, pageKind, path);
}
