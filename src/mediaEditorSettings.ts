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
import { MEDIA_EDITORS, type MediaEditor } from "./mediaEditors";

const [commands, setCommands] = createStore<Record<string, string>>({});
const commandRevision = new Map<string, number>();

/** Reactive: the configured command template for a registry entry (""=OS opener). */
export function mediaEditorCommand(settingKey: string): string {
  return commands[settingKey] ?? "";
}

/** Set + persist an editor's command template. */
export function setMediaEditorCommand(settingKey: string, value: string): void {
  const v = value.trim();
  commandRevision.set(settingKey, (commandRevision.get(settingKey) ?? 0) + 1);
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
    return (await detectMediaEditorCommand(ed)).command;
  } catch {
    return "";
  }
}

/** Probe on explicit Autodetect or first launch; a newer manual edit wins. */
export async function detectMediaEditorCommand(ed: MediaEditor): Promise<{ command: string; applied: boolean }> {
  const revision = commandRevision.get(ed.settingKey) ?? 0;
  const found = (await backend().detectMediaEditor(ed.id)).trim();
  if ((commandRevision.get(ed.settingKey) ?? 0) !== revision) {
    return { command: mediaEditorCommand(ed.settingKey), applied: false };
  }
  if (found) setMediaEditorCommand(ed.settingKey, found);
  return { command: found, applied: true };
}

/** Load all persisted editor commands at startup (default = empty = OS opener). */
export async function initMediaEditorSettings(): Promise<void> {
  await Promise.all(
    MEDIA_EDITORS.map(async (e) => {
      const revision = commandRevision.get(e.settingKey) ?? 0;
      try {
        const v = await backend().getAppString(e.settingKey, "");
        if ((commandRevision.get(e.settingKey) ?? 0) === revision) setCommands(e.settingKey, v || "");
      } catch {
        /* keep empty */
      }
    }),
  );
}
