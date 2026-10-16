import { activeWindow } from "./windowRealm";

declare global {
  interface WindowEventMap {
    "tine-new-workspace": Event;
  }
}

/** Route the command to the mounted switcher in the invoking window. */
export function openWorkspaceCreation(): void {
  activeWindow().dispatchEvent(new Event("tine-new-workspace"));
}
