import { renderEnglish } from "../i18n";
import { richPlain } from "../i18n/Rich";
import { settings } from "./settingDefinitions";
import { settingEntries } from "./settingsRegistry";
import type { SettingDescriptor } from "./settingsDescriptor";

/** The document is a build artifact of descriptors, including gated controls.
 * Installed packages are device data, documented by their registry below. */
export function generatedSettingsInventory(): string {
  const entries: SettingDescriptor[] = [...Object.values(settings), ...settingEntries().filter(entry => !Object.values(settings).some(setting => setting.id === entry.id))];
  const plain = (text: typeof entries[number]["label"]) =>
    (typeof text === "string" ? text : richPlain(renderEnglish(text))).replaceAll("|", "\\|").replaceAll("\n", " ");
  const lines = [
    "# Settings inventory", "",
    "Generated from the descriptors consumed by the Settings controls and search.",
    "Run `node scripts/generate-settings-inventory.mjs` after changing a descriptor;",
    "`node scripts/generate-settings-inventory.mjs --check` checks staleness.", "",
    "IDs are stable navigation targets (`data-setting-id`); labels resolve in the selected language.",
    "Search also accepts English terms of two or more characters and declared aliases. A single character matches only displayed-language text.", "",
    "The inventory includes gated controls (CM6, desktop, touch, Sharing) even where a build or device hides them.", "",
  ];
  for (const tab of new Set(entries.map(entry => entry.tab))) {
    lines.push(`<!-- tab:${tab} -->`, "", `## ${tab}`, "",
      "| Setting ID | Catalog key / package label | Label | Level |", "|---|---|---|---|");
    for (const entry of entries.filter(entry => entry.tab === tab)) {
      const key = typeof entry.label === "string" ? entry.label : entry.label.id;
      lines.push(`| \`${entry.id}\` | \`${key}\` | ${plain(entry.label)} | ${entry.level === "advanced" ? "Advanced" : "Basic"} |`);
    }
    lines.push("");
  }
  lines.push("## Device and registry entries", "",
    "In addition to the rows above, search and Themes use the same installed theme and signed community theme registries: each package contributes its name and description, with a distinct ID for its row. Built-in gallery cards use the gallery registry.", "",
    "Installed plugins contribute each manifest setting's label and description, plus the plugin name. Choosing a result opens that exact plugin version's settings form. Plugin and theme text is supplied by its author and stays verbatim in every interface language.", "");
  return lines.join("\n");
}
