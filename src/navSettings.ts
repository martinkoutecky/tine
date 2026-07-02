// Device-local navigation preferences (persisted in tine-settings.json via the
// generic app_bool backend, so they survive a restart).

import { createSignal } from "solid-js";
import { backend } from "./backend";

const KEY_REUSE_TABS = "nav_reuse_tabs";

const [reuseTabs, setReuseTabsSig] = createSignal(true);

/** Reactive: user navigations focus an already-open exact route instead of
 *  replacing the active tab / opening a duplicate. */
export const navReuseTabs = reuseTabs;

export function setNavReuseTabs(on: boolean): void {
  setReuseTabsSig(on);
  void backend().setAppBool(KEY_REUSE_TABS, on).catch(() => {});
}

/** Load the persisted navigation preference at startup. Default ON. */
export async function initNavSettings(): Promise<void> {
  try {
    setReuseTabsSig(await backend().getAppBool(KEY_REUSE_TABS, true));
  } catch {
    /* default on */
  }
}
