// Device-local, default-off experiment. The shipping identity is the only
// availability gate; stable builds ignore even a previously stored opt-in.
import { createSignal } from "solid-js";
import { APP_UPDATE_CHANNEL } from "../appIdentity";
import { backend } from "../backend";
import { loadPreference, writePreference } from "../preferenceWrites";

const KEY = "experimental_cm6_editor";
const [enabled, setEnabled] = createSignal(false);
export const cm6EditorAvailable = APP_UPDATE_CHANNEL === "beta";
export const cm6EditorEnabled = () => cm6EditorAvailable && enabled();

/** Apply to the next editor activation and persist through the existing
 * device preference queue. A failed write rolls back and reports the failure. */
export function setCm6EditorEnabled(on: boolean): void {
  if (!cm6EditorAvailable) return;
  writePreference(enabled, setEnabled, on,
    next => backend().setAppBool(KEY, next), "CodeMirror editor");
}

/** Stable builds neither read nor write this experimental preference. */
export function initCm6EditorSetting(): void {
  if (!cm6EditorAvailable) return;
  loadPreference(enabled, setEnabled,
    () => backend().getAppBool(KEY, false), value => value, "CodeMirror editor");
}
