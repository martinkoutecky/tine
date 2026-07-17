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

export function setRefClickZoom(on: boolean): void {
  setRefZoomSig(on);
  void backend().setAppBool(KEY_REF_ZOOM, on).catch(() => {});
}

export function setCopyIncludeSubtree(on: boolean): void {
  setIncludeSubtreeSig(on);
  void backend().setAppBool(KEY_SUBTREE, on).catch(() => {});
}
export function setCopyStripCollapsed(on: boolean): void {
  setStripCollapsedSig(on);
  void backend().setAppBool(KEY_COLLAPSED, on).catch(() => {});
}

/** Load the persisted preferences at startup. Tine defaults: include-subtree OFF,
 *  strip-collapsed ON (both differ from Logseq; revertible in Settings). */
export async function initCopySettings(): Promise<void> {
  try {
    setIncludeSubtreeSig(await backend().getAppBool(KEY_SUBTREE, false));
  } catch {
    /* default off */
  }
  try {
    setStripCollapsedSig(await backend().getAppBool(KEY_COLLAPSED, true));
  } catch {
    /* default on */
  }
  try {
    setRefZoomSig(await backend().getAppBool(KEY_REF_ZOOM, false));
  } catch {
    /* default off */
  }
}
