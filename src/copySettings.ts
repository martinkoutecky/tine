// Device-local copy/paste behavior preferences (persisted in tine-settings.json via
// the generic app_bool backend for atomic, WebView-independent state). Read once at
// startup by initCopySettings(); the signals drive both the copy logic
// (store.ts selectionMarkdown) and the Settings toggles.
//
// Both DIFFER from OG by default (Tine's preferred behavior), with a one-click
// revert to Logseq in Settings:
//   - copyIncludeSubtree: Tine default OFF (copy only the SELECTED blocks). OG always
//     copies a selected block's whole sub-tree → turn ON to match Logseq.
//   - copyStripCollapsed: Tine default ON (drop `collapsed::` from copied text — it's
//     UI state, not content). OG keeps it → turn OFF to match Logseq.

import { createSignal } from "solid-js";
import { backend } from "./backend";
import { writePreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";

const KEY_SUBTREE = "copy_include_subtree";
const KEY_COLLAPSED = "copy_strip_collapsed";
const KEY_REF_ZOOM = "ref_click_zoom";

const [includeSubtree, setIncludeSubtreeSig] = createSignal(false);
const [stripCollapsed, setStripCollapsedSig] = createSignal(true);
const [refZoom, setRefZoomSig] = createSignal(false);

/** Reactive: when copying a parent, also include its sub-blocks? OFF = Tine default
 *  (only the selected blocks); ON = Logseq behavior (whole sub-tree). */
export const copyIncludeSubtree = includeSubtree;
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
export function setCopyIncludeSubtree(on: boolean): void {
  writePreference(includeSubtree, setIncludeSubtreeSig, on, (next) => backend().setAppBool(KEY_SUBTREE, next), "copy subtree preference");
}
/** Apply now and queue a device-local write. Failure rolls back and toasts;
 * return does not confirm persistence. O(1) plus backend write. */
export function setCopyStripCollapsed(on: boolean): void {
  writePreference(stripCollapsed, setStripCollapsedSig, on, (next) => backend().setAppBool(KEY_COLLAPSED, next), "copy collapsed preference");
}

/** Load device preferences at startup: include-subtree and ref-click zoom OFF,
 * strip-collapsed ON. Failed reads toast and resolve. */
export async function initCopySettings(): Promise<void> {
  const subtreeRevision = preferenceRevision(includeSubtree);
  const collapsedRevision = preferenceRevision(stripCollapsed);
  const zoomRevision = preferenceRevision(refZoom);
  try {
    const value = await backend().getAppBool(KEY_SUBTREE, false);
    if (preferenceReadCurrent(includeSubtree, subtreeRevision)) { setIncludeSubtreeSig(value); seedPreference(includeSubtree); }
  } catch {
    pushToast("Could not load copy subtree preference.", "error");
  }
  try {
    const value = await backend().getAppBool(KEY_COLLAPSED, true);
    if (preferenceReadCurrent(stripCollapsed, collapsedRevision)) { setStripCollapsedSig(value); seedPreference(stripCollapsed); }
  } catch {
    pushToast("Could not load collapsed copy preference.", "error");
  }
  try {
    const value = await backend().getAppBool(KEY_REF_ZOOM, false);
    if (preferenceReadCurrent(refZoom, zoomRevision)) { setRefZoomSig(value); seedPreference(refZoom); }
  } catch {
    pushToast("Could not load block reference preference.", "error");
  }
}
