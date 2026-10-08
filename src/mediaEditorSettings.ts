// Device-local external-editor command templates (GH #38), persisted in
// tine-settings.json via the generic app_string backend so native launch actions
// and independent WebViews share one value. One command per media-editor registry
// entry, keyed by its `settingKey`. Read once at startup by
// initMediaEditorSettings(); the store drives both the "Edit in …" action and
// the Settings → Files rows. Empty = the OS default opener.
//
// A manual edit supersedes a pending startup read or autodetect for that key.
// Mirrors src/assetSettings.ts. See src/mediaEditors.ts for the registry.

import { createStore } from "solid-js/store";
import { backend } from "./backend";
import { advanceRevision, currentRevision, latestOwner, readOwned, revisionOwner } from "./owned";
import { MEDIA_EDITORS, type MediaEditor } from "./mediaEditors";
import { writePreference, seedPreference } from "./preferenceWrites";
import { pushToast } from "./toasts";

const [commands, setCommands] = createStore<Record<string, string>>({});
const commandKeys = new Map<string, object>();
const commandProbes = {};
function commandKey(settingKey: string): object {
  let key = commandKeys.get(settingKey);
  if (!key) { key = {}; commandKeys.set(settingKey, key); }
  return key;
}

/** Reactive: the configured command template for a registry entry (""=OS opener). */
export function mediaEditorCommand(settingKey: string): string {
  return commands[settingKey] ?? "";
}

/** Trim and optimistically persist the command; unregistered keys are accepted,
 * and write failure rolls back with a toast. */
export function setMediaEditorCommand(settingKey: string, value: string): void {
  const v = value.trim();
  advanceRevision(commandKey(settingKey));
  const read = commandReaders.get(settingKey) ?? (() => mediaEditorCommand(settingKey));
  commandReaders.set(settingKey, read);
  writePreference(read, (next) => setCommands(settingKey, next), v,
    (next) => backend().setAppString(settingKey, next), "media editor command");
}

const commandReaders = new Map<string, () => string>();

/** Resolve the launch command for an editor. Uses the user's configured template
 *  if set; otherwise runs a one-time autodetect probe (`detect_media_editor`) and,
 *  if it finds an install, persists it (so Settings → Files reflects it and we
 *  don't re-probe every launch). Empty result ⇒ the caller falls back to the OS
 *  opener. Without this, a first `/drawio` on a machine that has drawio installed
 *  but no command configured would open the SVG in the OS default image viewer
 *  (e.g. gwenview) instead of drawio (GH #38). */
export async function resolveMediaEditorCommand(ed: MediaEditor): Promise<string> {
  const existing = mediaEditorCommand(ed.settingKey);
  if (existing) return existing;
  if (!ed.detectable) return "";
  try {
    return (await detectMediaEditorCommand(ed)).command;
  } catch {
    return "";
  }
}

/** Run one native probe for ed.id; callers choose when. If this key changes
 * during the probe, return its current command with applied:false. Otherwise
 * return the trimmed result with applied:true, even when empty. A nonempty
 * result updates reactive state and starts a device-local settings
 * write whose failure restores the previous command and shows an error. Probe failure rejects. O(1) backend calls
 * plus native probe latency; applied does not promise persistence. */
export async function detectMediaEditorCommand(ed: MediaEditor): Promise<{ command: string; applied: boolean }> {
  const key = commandKey(ed.settingKey);
  const owner = latestOwner(commandProbes, ed.settingKey, revisionOwner(key, currentRevision(key)));
  const result = await readOwned(owner, backend().detectMediaEditor(ed.id));
  if (result.kind === "stale") {
    return { command: mediaEditorCommand(ed.settingKey), applied: false };
  }
  const found = result.value.trim();
  if (found) setMediaEditorCommand(ed.settingKey, found);
  return { command: found, applied: true };
}

/** Load commands at startup (empty = OS opener). Failed reads toast and resolve;
 * a manual change supersedes its pending read. */
export async function initMediaEditorSettings(): Promise<void> {
  await Promise.all(
    MEDIA_EDITORS.map(async (e) => {
      const key = commandKey(e.settingKey);
      const revision = currentRevision(key);
      try {
        const v = await backend().getAppString(e.settingKey, "");
        if (revisionOwner(key, revision)()) {
          setCommands(e.settingKey, v || "");
          const read = commandReaders.get(e.settingKey);
          if (read) seedPreference(read);
        }
      } catch {
        pushToast(`Could not load ${e.id} editor command.`, "error");
      }
    }),
  );
}
