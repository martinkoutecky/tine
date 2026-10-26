// Right-sidebar card replacement, split out of ui.ts (size ratchet).
import { rightSidebar, setRightSidebar, sidebarItemKey, type SidebarItem } from "./ui";

/** OG `sidebar-replace-block!`: a breadcrumb click swaps this card for its target
 * in place. Another card already showing that target is dropped, so one target
 * never has two live sidebar slots. O(sidebar items). */
export function replaceSidebarItem(from: SidebarItem, to: SidebarItem): void {
  const items = rightSidebar();
  if (!items.includes(from)) return;
  const key = sidebarItemKey(to);
  setRightSidebar(items.flatMap((item) => item === from ? [to] : sidebarItemKey(item) === key ? [] : [item]));
}
