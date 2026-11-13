import { msg } from "./i18n";
// Device-local copy/paste behavior preferences (persisted in tine-settings.json via
// the generic app_bool backend for atomic, WebView-independent state). Read once at
// startup by initCopySettings(); the signals drive both the copy logic
// (document's selectionMarkdown) and the Settings toggles.
//
// copyStripCollapsed: Tine default ON (drop `collapsed::` from copied text — it's
//     UI state, not content). OG keeps it → turn OFF to match Logseq.

import { createSignal } from "solid-js";
import { backend } from "./backend";
import { writePreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";

const KEY_COLLAPSED = "copy_strip_collapsed";
const KEY_REF_ZOOM = "ref_click_zoom";

// Each default is spelled once: the initial signal and the startup read share it.
const DEFAULT_STRIP_COLLAPSED = true;
const DEFAULT_REF_ZOOM = false;

const [stripCollapsed, setStripCollapsedSig] = createSignal(DEFAULT_STRIP_COLLAPSED);
const [refZoom, setRefZoomSig] = createSignal(DEFAULT_REF_ZOOM);

/** Reactive: strip `collapsed::` from copied text? ON = Tine default (cleaner
 *  paste); OFF = Logseq (keeps it). `id::` is always stripped regardless. */
export const copyStripCollapsed = stripCollapsed;
/** Reactive: plain-click an inline block ref → zoom into it (OG) vs scroll to it
 *  in context (Tine default OFF). */
export const refClickZoom = refZoom;

/** Apply now and queue a device-local write. Failure rolls back and toasts;
 * return does not confirm persistence. O(1) plus backend write. */
export function setRefClickZoom(on: boolean): void {
  writePreference(refZoom, setRefZoomSig, on, (next) => backend().setAppBool(KEY_REF_ZOOM, next), "block reference click behavior");
}

/** Apply now and queue a device-local write. Failure rolls back and toasts;
 * return does not confirm persistence. O(1) plus backend write. */
export function setCopyStripCollapsed(on: boolean): void {
  writePreference(stripCollapsed, setStripCollapsedSig, on, (next) => backend().setAppBool(KEY_COLLAPSED, next), "copy collapsed preference");
}

/** Load device preferences at startup: ref-click zoom OFF,
 * strip-collapsed ON. Failed reads toast and resolve. */
export async function initCopySettings(): Promise<void> {
  const collapsedRevision = preferenceRevision(stripCollapsed);
  const zoomRevision = preferenceRevision(refZoom);
  try {
    const value = await backend().getAppBool(KEY_COLLAPSED, DEFAULT_STRIP_COLLAPSED);
    if (preferenceReadCurrent(stripCollapsed, collapsedRevision)) { setStripCollapsedSig(value); seedPreference(stripCollapsed); }
  } catch {
    pushToast(msg("settings-could-not-load-collapsed-copy-preference"), "error");
  }
  try {
    const value = await backend().getAppBool(KEY_REF_ZOOM, DEFAULT_REF_ZOOM);
    if (preferenceReadCurrent(refZoom, zoomRevision)) { setRefZoomSig(value); seedPreference(refZoom); }
  } catch {
    pushToast(msg("settings-could-not-load-block-reference-preference"), "error");
  }
}
