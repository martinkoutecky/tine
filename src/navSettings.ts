import { msg, renderEnglish } from "./i18n";
// Device-local view/navigation preferences (persisted in tine-settings.json via the
// generic app_bool backend, so they survive a restart; localStorage does not in the
// Tine app on WebKitGTK).

import { createSignal } from "solid-js";
import { backend } from "./backend";
import { writePreference, loadPreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";

const KEY_REUSE_TABS = "nav_reuse_tabs";
const KEY_SHOW_WORKSPACES = "show_workspaces";
const [showWorkspacesSig, setShowWorkspacesSig] = createSignal(true);

/** Reactive device preference for saved-Workspace controls; reading is O(1). */
export const showWorkspaces = showWorkspacesSig;

/** Show or hide controls without changing workspace data. One device write;
 * failure rolls back and toasts through the shared preference writer. */
export function setShowWorkspaces(on: boolean): void {
  writePreference(showWorkspacesSig, setShowWorkspacesSig, on, next => backend().setAppBool(KEY_SHOW_WORKSPACES, next), msg("toast-save-workspaces-failed"));
}

const KEY_SHOW_HELP_BUTTON = "show_help_button";
const [showHelpButtonSig, setShowHelpButtonSig] = createSignal(true);

/** Reactive device preference for the corner "?" button (GH #926); reading is O(1).
 *  Off hides only the button: the help menu still opens from the palette and
 *  Settings → Help & diagnostics lists the same entries. */
export const showHelpButton = showHelpButtonSig;

/** One device write; failure rolls back and toasts through the shared preference writer. */
export function setShowHelpButton(on: boolean): void {
  writePreference(showHelpButtonSig, setShowHelpButtonSig, on, next => backend().setAppBool(KEY_SHOW_HELP_BUTTON, next), msg("toast-save-help-button-failed"));
}

const KEY_QUERY_TEXT_OPEN = "query_text_open";

const KEY_MOBILE_NAV_BOTTOM = "mobile_navigation_at_bottom";

const KEY_SEARCH_OPENS_HERE = "search_results_open_in_this_tab";

const [reuseTabs, setReuseTabsSig] = createSignal(true);
const [queryTextOpenSig, setQueryTextOpenSig] = createSignal(false);
const [mobileNavBottom, setMobileNavBottomSig] = createSignal(false);
const [searchOpensHere, setSearchOpensHereSig] = createSignal(false);

/** Reactive: a plain click or Enter on a Search-tab result navigates the Search
 *  tab itself (GH #796) instead of opening a new tab beside it (GH #416, the
 *  default). Modifier and middle-click destinations are the shared link contract
 *  either way. Device-local, survives a restart. */
export const searchResultsOpenInThisTab = searchOpensHere;

/** Apply now and queue a device-local write; failure rolls back and toasts. O(1) plus backend write. */
export function setSearchResultsOpenInThisTab(on: boolean): void {
  writePreference(searchOpensHere, setSearchOpensHereSig, on, (next) => backend().setAppBool(KEY_SEARCH_OPENS_HERE, next), msg("toast-save-search-results-failed"));
}

/** Test seam: set the destination without persisting. */
export function resetSearchResultsOpenInThisTabForTests(on = false): void {
  setSearchOpensHereSig(on);
  seedPreference(searchOpensHere);
}

/** Reactive: on a phone or tablet app, Sidebar / Search / Back / Forward sit in a
 *  bottom row (GH #598) instead of the top bar (GH #716). Default off, device-local,
 *  survives a restart; desktop ignores it. */
export const mobileNavigationAtBottom = mobileNavBottom;

/** Apply now and queue a device-local write; failure rolls back and toasts. O(1) plus backend write. */
export function setMobileNavigationAtBottom(on: boolean): void {
  writePreference(mobileNavBottom, setMobileNavBottomSig, on, (next) => backend().setAppBool(KEY_MOBILE_NAV_BOTTOM, next), msg("toast-save-navigation-buttons-failed"));
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
  writePreference(queryTextOpenSig, setQueryTextOpenSig, on, (next) => backend().setAppBool(KEY_QUERY_TEXT_OPEN, next), msg("toast-save-query-text-failed"));
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
  writePreference(reuseTabs, setReuseTabsSig, on, (next) => backend().setAppBool(KEY_REUSE_TABS, next), msg("toast-save-reuse-tabs-failed"));
}

/** Load the device preference at startup (default ON); read failure toasts and resolves. */
export async function initNavSettings(): Promise<void> {
  loadPreference(showWorkspacesSig, setShowWorkspacesSig, () => backend().getAppBool(KEY_SHOW_WORKSPACES, true), v => v, renderEnglish(msg("settings-show-workspaces")));
  loadPreference(showHelpButtonSig, setShowHelpButtonSig, () => backend().getAppBool(KEY_SHOW_HELP_BUTTON, true), v => v, renderEnglish(msg("settings-show-help-button")));
  loadPreference(queryTextOpenSig, setQueryTextOpenSig, () => backend().getAppBool(KEY_QUERY_TEXT_OPEN, false), (v) => v, "query text preference");
  loadPreference(mobileNavBottom, setMobileNavBottomSig, () => backend().getAppBool(KEY_MOBILE_NAV_BOTTOM, false), (v) => v, "navigation buttons preference");
  loadPreference(searchOpensHere, setSearchOpensHereSig, () => backend().getAppBool(KEY_SEARCH_OPENS_HERE, false), (v) => v, "search results preference");
  const revision = preferenceRevision(reuseTabs);
  try {
    const value = await backend().getAppBool(KEY_REUSE_TABS, true);
    if (preferenceReadCurrent(reuseTabs, revision)) { setReuseTabsSig(value); seedPreference(reuseTabs); }
  } catch {
    pushToast(msg("settings-could-not-load-tab-reuse-preference"), "error");
  }
}
