import { activeWindow } from "./windowRealm";
import { mobileDrawerMode } from "./mobileDrawers";
import { setLeftSidebarOpen, sidebarOpen } from "./ui";

declare global {
  interface WindowEventMap {
    "tine-new-workspace": Event;
  }
}

/** Route the command to the mounted switcher in the invoking window. In drawer
 * mode the left drawer's switcher takes it (the bar's compact switcher is hidden
 * on phones, #926), so the drawer opens first. */
export function openWorkspaceCreation(): void {
  if (mobileDrawerMode() && !sidebarOpen()) setLeftSidebarOpen(true);
  activeWindow().dispatchEvent(new Event("tine-new-workspace"));
}
