import { msg } from "../i18n";
// Device-local, default-off experiment. The shipping identity is the only
// availability gate; stable builds ignore even a previously stored opt-in.
import { createSignal } from "solid-js";
import { APP_UPDATE_CHANNEL } from "../appIdentity";
import { backend } from "../backend";
import { revisionOwner, writeOwned } from "../owned";
import { loadPreference, preferenceRevision, preferenceChangedElsewhere, writePreference } from "../preferenceWrites";

const KEY = "experimental_cm6_editor";
const CHANGED = "cm6-editor-setting-changed";
const thisWindow = crypto.randomUUID();
let changeListener: Promise<void> | undefined;
/** Every window of the app has its own copy of this signal, so a choice made in
 * one window reaches the others through this event; each re-reads the file once
 * its own pending writes settle (the pinyin preference does the same). */
function listenForChanges(): Promise<void> {
  return changeListener ??= (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      await listen<{ origin: string }>(CHANGED, event => {
        if (event.payload.origin === thisWindow) return;
        preferenceChangedElsewhere(enabled, () => void loadStored());
      });
    } catch { /* Published pages and tests have no native event bus. */ }
  })();
}
const [enabled, setEnabled] = createSignal(false);
const [loading, setLoading] = createSignal(false);
let loadsInFlight = 0;
/** True while a read of the stored choice is in flight. An editor chooses its
 * engine once, when it mounts, so one that would mount during a read waits for
 * it (Block `Editor`); otherwise a slow reply (a big graph keeping the backend
 * busy at launch) leaves the first editor on the default until the next one. */
export const cm6EditorLoading = loading;
export const cm6EditorAvailable = APP_UPDATE_CHANNEL === "beta";
export const cm6EditorEnabled = () => cm6EditorAvailable && enabled();

/** Apply to the next editor activation and persist through the existing
 * device preference queue. A failed write rolls back and reports the failure. */
export function setCm6EditorEnabled(on: boolean): void {
  if (!cm6EditorAvailable) return;
  writePreference(enabled, setEnabled, on,
    async next => {
      await writeOwned(revisionOwner(enabled, preferenceRevision(enabled)), backend().setAppBool(KEY, next));
      try {
        const { emit } = await import("@tauri-apps/api/event");
        await emit(CHANGED, { origin: thisWindow });
      } catch { /* The file is written; other windows read it at their next start. */ }
    }, msg("toast-save-codemirror-failed"));
}

/** Stable builds neither read nor write this experimental preference. The
 * promise settles once the stored choice has landed (or failed to load): an
 * editor mounted earlier keeps the default, so a window that mounts one before
 * its first user edit (Quick Capture) waits for this. */
export function initCm6EditorSetting(): Promise<void> {
  if (!cm6EditorAvailable) return Promise.resolve();
  void listenForChanges();
  return loadStored();
}

function loadStored(): Promise<void> {
  loadsInFlight++;
  setLoading(true);
  const landed = loadPreference(enabled, setEnabled,
    () => backend().getAppBool(KEY, false), value => value, "CodeMirror editor");
  return landed.finally(() => { if (--loadsInFlight === 0) setLoading(false); });
}
