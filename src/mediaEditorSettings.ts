// Device-local external-editor command templates (GH #38), persisted in
// tine-settings.json via the generic app_string backend so they survive a
// restart (WebKitGTK localStorage does not). One command per media-editor
// registry entry, keyed by its `settingKey`. Read once at startup by
// initMediaEditorSettings(); the store drives both the "Edit in …" action and
// the Settings → Files rows. Empty = the OS default opener.
//
// Mirrors src/assetSettings.ts. See src/mediaEditors.ts for the registry.

import { createStore } from "solid-js/store";
import { backend } from "./backend";
import { MEDIA_EDITORS, type MediaEditor } from "./mediaEditors";

const [commands, setCommands] = createStore<Record<string, string>>({});

/** Reactive: the configured command template for a registry entry (""=OS opener). */
export function mediaEditorCommand(settingKey: string): string {
  return commands[settingKey] ?? "";
}

/** Set + persist an editor's command template. */
export function setMediaEditorCommand(settingKey: string, value: string): void {
  const v = value.trim();
  setCommands(settingKey, v);
  void backend().setAppString(settingKey, v).catch(() => {});
}

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
    const found = (await backend().detectMediaEditor(ed.id)).trim();
    if (found) {
      setMediaEditorCommand(ed.settingKey, found);
      return found;
    }
  } catch {
    /* fall through to OS opener */
  }
  return "";
}

/** Load all persisted editor commands at startup (default = empty = OS opener). */
export async function initMediaEditorSettings(): Promise<void> {
  await Promise.all(
    MEDIA_EDITORS.map(async (e) => {
      try {
        const v = await backend().getAppString(e.settingKey, "");
        setCommands(e.settingKey, v || "");
      } catch {
        /* keep empty */
      }
    }),
  );
}
