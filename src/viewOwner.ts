import { viewOwnerPath } from "./routerBridge";
import type { PageKind } from "./types";
import type { SidebarItem } from "./ui";

/** Same-owner check of both sidebar open handlers, over the router's one
 *  answer to "which file does this view show": a pathful result never evicts
 *  the name-only view of the same file, and a different same-name file still
 *  replaces it. */
export function sameSidebarOwner(item: SidebarItem, name: string, pageKind: PageKind, path: string | undefined): boolean {
  const itemName = item.kind === "page" ? item.name : item.page;
  if (itemName !== name || item.pageKind !== pageKind) return false;
  return viewOwnerPath(itemName, item.pageKind, item.path) === viewOwnerPath(name, pageKind, path);
}
