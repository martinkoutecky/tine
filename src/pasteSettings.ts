// Device-local paste preference, shared by both block editor engines.
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { msg, t } from "./i18n";
import { loadPreference, writePreference } from "./preferenceWrites";

const KEY = "auto_embed_video_links";
const DEFAULT = true;
const [enabled, setEnabled] = createSignal(DEFAULT);

export const autoEmbedVideoLinks = enabled;

/** Apply now and queue the existing guarded device preference write. A failed
 * write rolls back and toasts; return does not confirm persistence. */
export function setAutoEmbedVideoLinks(on: boolean): void {
  writePreference(enabled, setEnabled, on, next => backend().setAppBool(KEY, next),
    msg("toast-save-auto-embed-failed"));
}

/** Restore the device choice without overwriting a newer local change. */
export function initPasteSettings(): Promise<void> {
  return loadPreference(enabled, setEnabled, () => backend().getAppBool(KEY, DEFAULT),
    value => value, t("settings-auto-embed-video-links"));
}
