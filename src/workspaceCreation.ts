import { activeWindow } from "./windowRealm";
import { mobileDrawerMode } from "./mobileDrawers";
import { setLeftSidebarOpen, sidebarOpen } from "./ui";

declare global {
  interface WindowEventMap {
    "tine-open-workspace-menu": CustomEvent<{ create: boolean }>;
  }
}

/** Route creation to exactly one visible switcher in the invoking window.
 * Drawer mode opens the sidebar first. If toolbar customization hid the
 * compact control, an unclaimed request reveals the sidebar and retries once.
 * O(1) window events, no storage or graph work. */
function requestWorkspaceMenu(create: boolean): void {
  if (mobileDrawerMode() && !sidebarOpen()) setLeftSidebarOpen(true);
  const dispatch = () => {
    const event = new CustomEvent("tine-open-workspace-menu", { cancelable: true, detail: { create } });
    activeWindow().dispatchEvent(event);
    return event.defaultPrevented;
  };
  if (!dispatch() && !sidebarOpen()) { setLeftSidebarOpen(true); dispatch(); }
}

/** Open the existing workspace picker, revealing its sidebar when needed. */
export function openWorkspaceMenu(): void { requestWorkspaceMenu(false); }

/** Open that picker with its existing New workspace form. */
export function openWorkspaceCreation(): void { requestWorkspaceMenu(true); }
