import type { Msg } from "../i18n";
import type { SettingsTabId } from "../ui";
import type { StyleKnob } from "../styleKnobs";
import type { MediaEditor } from "../mediaEditors";
import type { ManagedPlugin } from "../plugins/manager";
import type { InstalledTheme } from "../themes/manager";
import type { RegistryTheme } from "../plugins/registry";
import type { GalleryTheme } from "../styles/themes";

/** One setting's identity and display/search vocabulary. Localized text stays
 * unresolved until a reactive consumer renders it. Package text is verbatim. */
export type SettingDescriptor = {
  id: string;
  tab: SettingsTabId;
  label: string | Msg;
  description?: string | Msg;
  /** Supplemental vocabulary for the description, without name-match ranking. */
  searchDescription?: string | Msg;
  keywords?: string | Msg;
  aliases?: string[];
  level?: "advanced";
  touchOnly?: true;
  /** Open the installed plugin's settings form before finding its row. */
  pluginKey?: string;
};

export function knobSetting(knob: Exclude<StyleKnob, { type: "heading" }>): SettingDescriptor {
  return { id: `knob.${knob.id}`, tab: "appearance", label: knob.title,
    description: knob.description, level: "advanced" };
}

export function mediaEditorSetting(editor: MediaEditor): SettingDescriptor {
  return { id: `media-editor.${editor.id}`, tab: "files", label: editor.settingLabel, level: "advanced" };
}

export function installedThemeSetting(theme: InstalledTheme): SettingDescriptor {
  return { id: `theme.installed.${theme.key}`, tab: "themes", label: theme.manifest.name,
    description: theme.manifest.description };
}

export function communityThemeSetting(theme: RegistryTheme): SettingDescriptor {
  return { id: `theme.community.${theme.id}`, tab: "themes", label: theme.name, description: theme.description };
}

export function galleryThemeSetting(theme: Pick<GalleryTheme, "id" | "name" | "author">): SettingDescriptor {
  return { id: `theme.gallery.${theme.id}`, tab: "themes", label: theme.name, description: theme.author };
}

export function pluginSetting(plugin: ManagedPlugin, definition: NonNullable<ManagedPlugin["manifest"]["settings"]>[number]): SettingDescriptor {
  const pluginKey = `${plugin.manifest.id}@${plugin.manifest.version}`;
  return { id: `plugin.${pluginKey}/${definition.key}`, tab: "plugins", label: definition.label,
    description: definition.description, keywords: plugin.manifest.name, pluginKey };
}
