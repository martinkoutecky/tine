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
import { MEDIA_EDITORS } from "./mediaEditors";

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
