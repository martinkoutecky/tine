// Device-local navigation preferences (persisted in tine-settings.json via the
// generic app_bool backend, so they survive a restart).

import { createSignal } from "solid-js";
import { backend } from "./backend";
import { writePreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";

const KEY_REUSE_TABS = "nav_reuse_tabs";

const [reuseTabs, setReuseTabsSig] = createSignal(true);

/** Reactive: user navigations focus an already-open exact route instead of
 *  replacing the active tab / opening a duplicate. */
export const navReuseTabs = reuseTabs;

/** Apply now and queue a device-local write. Failure rolls back and toasts;
 * return does not confirm persistence. O(1) plus backend write. */
export function setNavReuseTabs(on: boolean): void {
  writePreference(reuseTabs, setReuseTabsSig, on, (next) => backend().setAppBool(KEY_REUSE_TABS, next), "tab reuse preference");
}

/** Load the device preference at startup (default ON); read failure toasts and resolves. */
export async function initNavSettings(): Promise<void> {
  const revision = preferenceRevision(reuseTabs);
  try {
    const value = await backend().getAppBool(KEY_REUSE_TABS, true);
    if (preferenceReadCurrent(reuseTabs, revision)) { setReuseTabsSig(value); seedPreference(reuseTabs); }
  } catch {
    pushToast("Could not load tab reuse preference.", "error");
  }
}
