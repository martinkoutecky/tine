import { settings } from "./settingDefinitions";
import { KNOBS } from "../styleKnobs";
import { MEDIA_EDITORS } from "../mediaEditors";
import { installedThemes } from "../themes/manager";
import { galleryThemes } from "../themeGallery";
import { COMMUNITY_REGISTRY_ENABLED, communityThemes } from "../plugins/registry";
import { installedPlugins } from "../plugins/manager";
import { communityThemeSetting, galleryThemeSetting, installedThemeSetting, knobSetting, mediaEditorSetting, pluginSetting, type SettingDescriptor } from "./settingsDescriptor";

/** Reactive registry snapshot. These are the same registries and descriptor
 * adapters consumed by their controls, including installs made while open. */
export function settingEntries(): SettingDescriptor[] {
  return [
    ...Object.values(settings),
    ...KNOBS.flatMap(knob => knob.type === "heading" ? [] : [knobSetting(knob)]),
    ...MEDIA_EDITORS.map(mediaEditorSetting),
    ...galleryThemes.map(galleryThemeSetting),
    ...installedThemes().map(installedThemeSetting),
    ...(COMMUNITY_REGISTRY_ENABLED ? communityThemes().map(communityThemeSetting) : []),
    ...installedPlugins().flatMap(plugin => (plugin.manifest.settings ?? []).map(definition => pluginSetting(plugin, definition))),
  ];
}
