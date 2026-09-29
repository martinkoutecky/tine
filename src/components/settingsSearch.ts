// Settings search vocabulary and matching contract: each visible setting has
// one label/description and optionally aliases; search matches all terms within
// that entry. Cost O(entries × query length), no I/O or failure.
import type { SettingsTabId } from "../ui";
type Tab = SettingsTabId;

export type SettingSearchEntry = { tab: Tab; label: string; description: string; aliases?: string[]; level?: "advanced" };
export const SETTING_SEARCH: SettingSearchEntry[] = [
  { tab: "appearance", label: "Theme", description: "light dark system gallery colors" },
  { tab: "appearance", label: "Accent color", description: "interface highlight color" },
  { tab: "appearance", label: "Interface size", description: "zoom scale Ctrl scroll" },
  { tab: "appearance", label: "Wide mode", description: "reading width" },
  { tab: "appearance", label: "Document mode", description: "hide bullets prose" },
  { tab: "appearance", label: "Document-mode Enter creates a new block", description: "Enter Shift Enter internal newline config" },
  { tab: "appearance", label: "Show brackets", description: "page references config shortcut" },
  { tab: "appearance", label: "Typographic replacements", description: "arrows dashes glyphs" },
  { tab: "appearance", label: "Auto-pair brackets & quotes", description: "closers selections backspace" },
  { tab: "appearance", label: "Space after inserting a reference", description: "page block autocomplete spacing" },
  { tab: "appearance", label: "Dim in focus mode", description: "inactive blocks" },
  { tab: "appearance", label: "Load local-file images", description: "absolute paths permission security" },
  { tab: "appearance", label: "Smooth scrolling (experimental)", description: "animated journal scrolling WebKit", aliases: ["scroll animation"], level: "advanced" },
  { tab: "appearance", label: "System title bar & window controls", description: "native frame chrome" },
  { tab: "editor", label: "File format", description: "new pages Markdown Org" },
  { tab: "editor", label: "Logical outdenting", description: "Shift Tab following siblings Roam config" },
  { tab: "editor", label: "Link autocomplete default", description: "OG adaptive existing typed page tag completion", level: "advanced" },
  { tab: "editor", label: "Switch to an already-open tab when navigating", description: "reuse tabs", level: "advanced" },
  { tab: "editor", label: "Learn Ctrl+K choices", description: "adaptive launcher ranking reset history", level: "advanced" },
  { tab: "editor", label: "Spell checker", description: "dictionaries languages spelling" },
  { tab: "editor", label: "Copy a parent block's sub-blocks", description: "clipboard subtree", level: "advanced" },
  { tab: "editor", label: "Strip collapsed:: when copying", description: "clipboard properties", level: "advanced" },
  { tab: "editor", label: "Click a block reference to zoom in", description: "reference navigation" },
  { tab: "journals", label: "Journal date format", description: "display titles" },
  { tab: "journals", label: "First day of week", description: "calendar Monday Sunday" },
  { tab: "journals", label: "Carry-over", description: "buttons context header last days" },
  { tab: "journals", label: "Task workflow", description: "TODO DOING NOW LATER" },
  { tab: "journals", label: "Time tracking", description: "LOGBOOK clock" },
  { tab: "journals", label: "New-journal template", description: "default journal template" },
  { tab: "journals", label: "Quick-capture Enter key", description: "capture submit new block", level: "advanced" },
  { tab: "journals", label: "Agenda window", description: "scheduled deadline days" },
  { tab: "files", label: "New asset filename", description: "paste drag media names" },
  { tab: "files", label: "Watch for external edits", description: "inotify polling network filesystem" },
  { tab: "files", label: "Diagram editors", description: "drawio Excalidraw commands", level: "advanced" },
  { tab: "backups", label: "Snapshots to keep", description: "recovery retention conflicts" },
  { tab: "graph", label: "Graph", description: "folder export publish" },
  { tab: "graph", label: "Home page", description: "home start startup page" },
  { tab: "diagnostics", label: "Help & diagnostics", description: "diagnostic report crash unclean exit save slow privacy help improve parser divergences anonymize" },
  { tab: "shortcuts", label: "Keyboard shortcuts", description: "key bindings commands remap" },
  { tab: "about", label: "About", description: "version licenses updates" },
];

/** Match all query words against one entry's label, description and aliases.
 * O(entry text × query words), no I/O or failure. */
export function settingMatches(entry: SettingSearchEntry, query: string): boolean {
  const terms = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
  const haystack = [entry.label, entry.description, ...(entry.aliases ?? [])].join(" ").toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

/** Whether a tab has an advanced setting matching a nonempty query. Used to
 * reveal that section while searching. O(entries × query length), no I/O. */
export function advancedMatch(tab: Tab, query: string): boolean {
  return !!query.trim() && SETTING_SEARCH.some((entry) => entry.tab === tab && entry.level === "advanced" && settingMatches(entry, query));
}
