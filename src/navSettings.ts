import { msg } from "./i18n";
// Device-local view/navigation preferences (persisted in tine-settings.json via the
// generic app_bool backend, so they survive a restart; localStorage does not in the
// Tine app on WebKitGTK).

import { createSignal } from "solid-js";
import { backend } from "./backend";
import { writePreference, loadPreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";

const KEY_REUSE_TABS = "nav_reuse_tabs";

const KEY_QUERY_TEXT_OPEN = "query_text_open";

const KEY_MOBILE_NAV_BOTTOM = "mobile_navigation_at_bottom";

const [reuseTabs, setReuseTabsSig] = createSignal(true);
const [queryTextOpenSig, setQueryTextOpenSig] = createSignal(false);
const [mobileNavBottom, setMobileNavBottomSig] = createSignal(false);

/** Reactive: on a phone or tablet app, Sidebar / Search / Back / Forward sit in a
 *  bottom row (GH #598) instead of the top bar (GH #716). Default off, device-local,
 *  survives a restart; desktop ignores it. */
export const mobileNavigationAtBottom = mobileNavBottom;

/** Apply now and queue a device-local write; failure rolls back and toasts. O(1) plus backend write. */
export function setMobileNavigationAtBottom(on: boolean): void {
  writePreference(mobileNavBottom, setMobileNavBottomSig, on, (next) => backend().setAppBool(KEY_MOBILE_NAV_BOTTOM, next), "navigation buttons preference");
}

/** Test seam: set the placement without persisting. */
export function resetMobileNavigationAtBottomForTests(on = false): void {
  setMobileNavBottomSig(on);
  seedPreference(mobileNavBottom);
}

/** Reactive: the query sheet's "Edit as text" pane is remembered open (GH #619
 *  item 4; default closed, device-local, survives a restart). */
export const queryTextOpen = queryTextOpenSig;

/** Apply now and queue a device-local write; failure rolls back and toasts. O(1) plus backend write. */
export function setQueryTextOpen(on: boolean): void {
  writePreference(queryTextOpenSig, setQueryTextOpenSig, on, (next) => backend().setAppBool(KEY_QUERY_TEXT_OPEN, next), "query text preference");
}

/** Test seam: set the remembered state without persisting. */
export function resetQueryTextOpenForTests(on = false): void {
  setQueryTextOpenSig(on);
  seedPreference(queryTextOpenSig);
}

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
  loadPreference(queryTextOpenSig, setQueryTextOpenSig, () => backend().getAppBool(KEY_QUERY_TEXT_OPEN, false), (v) => v, "query text preference");
  loadPreference(mobileNavBottom, setMobileNavBottomSig, () => backend().getAppBool(KEY_MOBILE_NAV_BOTTOM, false), (v) => v, "navigation buttons preference");
  const revision = preferenceRevision(reuseTabs);
  try {
    const value = await backend().getAppBool(KEY_REUSE_TABS, true);
    if (preferenceReadCurrent(reuseTabs, revision)) { setReuseTabsSig(value); seedPreference(reuseTabs); }
  } catch {
    pushToast(msg("settings-could-not-load-tab-reuse-preference"), "error");
  }
}
