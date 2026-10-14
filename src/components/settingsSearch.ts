// Settings search vocabulary and matching contract: each visible setting has
// one label/description and optionally aliases; search matches all terms within
// that entry. Cost O(entries × query length), no I/O or failure.
import { KNOBS } from "../styleKnobs";
import { msg, render, renderEnglish, type Msg } from "../i18n";
import type { SettingsTabId } from "../ui";
type Tab = SettingsTabId;

export type SettingSearchEntry = { tab: Tab; label: string | Msg; description: string | Msg; keywords?: string | Msg; aliases?: string[]; level?: "advanced" };
export const SETTING_SEARCH: SettingSearchEntry[] = [
  { tab: "appearance", label: msg("settings-search-language"), description: msg("settings-search-interface-translation-locale-menus"), aliases: ["语言", "中文", "Chinese", "English"] },
  { tab: "appearance", label: msg("settings-search-theme-mode"), description: msg("settings-search-light-dark-system") },
  { tab: "appearance", label: msg("settings-search-style"), description: msg("settings-search-typography-journal-headings-presentation-theme") },
  { tab: "appearance", label: msg("settings-search-color-scheme"), description: msg("settings-search-default-nord-solarized-gruvbox-soft-medium-light") },
  // Fine-tuning: one entry per knob in the table, so a new knob is searchable with no code here.
  ...KNOBS.filter((knob) => knob.type !== "heading").map((knob): SettingSearchEntry => ({
    tab: "appearance",
    label: knob.title,
    description: msg("settings-search-fine-tuning-style-knob-visual-detail"),
    keywords: knob.description,
  })),
  { tab: "appearance", label: msg("settings-search-edit-custom-css"), description: msg("settings-search-logseq-custom-css-stylesheet-theming-tokens-css-variables") },
  { tab: "appearance", label: msg("settings-search-disable-custom-css"), description: msg("settings-search-safe-mode-broken-stylesheet-unreadable-session") },
  { tab: "appearance", label: msg("settings-search-developer-tools"), description: msg("settings-search-inspector-inspect-element-styles-devtools") },
  { tab: "appearance", label: msg("settings-search-accent-color"), description: msg("settings-search-interface-highlight-color") },
  { tab: "appearance", label: msg("settings-search-interface-size"), description: msg("settings-search-zoom-scale-ctrl-scroll") },
  { tab: "appearance", label: msg("settings-search-wide-mode"), description: msg("settings-search-reading-width") },
  { tab: "appearance", label: msg("settings-search-standard-page-width"), description: msg("settings-search-reading-column-pixels-cap-reset"), aliases: ["narrow width"], level: "advanced" },
  { tab: "appearance", label: msg("settings-search-wide-page-width"), description: msg("settings-search-fill-pane-custom-pixels-cap"), aliases: ["wide mode width"], level: "advanced" },
  { tab: "appearance", label: msg("settings-search-document-mode"), description: msg("settings-search-hide-bullets-prose") },
  { tab: "appearance", label: msg("settings-search-document-mode-enter-creates-a-new-block"), description: msg("settings-search-enter-shift-enter-internal-newline-config") },
  { tab: "appearance", label: msg("settings-search-show-brackets"), description: msg("settings-search-page-references-config-shortcut") },
  { tab: "appearance", label: msg("settings-search-typographic-replacements"), description: msg("settings-search-arrows-dashes-glyphs") },
  { tab: "appearance", label: msg("settings-search-wrap-code-lines"), description: msg("settings-search-code-blocks-line-wrapping-long-lines") },
  { tab: "appearance", label: msg("setting-sticky-parents"), description: msg("setting-sticky-parents-hint"), aliases: ["sticky", "ancestors", "bullets", "scroll"] },
  { tab: "appearance", label: msg("settings-search-auto-pair-brackets-quotes"), description: msg("settings-search-closers-selections-backspace") },
  { tab: "appearance", label: msg("settings-search-space-after-inserting-a-reference"), description: msg("settings-search-page-block-autocomplete-spacing") },
  { tab: "appearance", label: msg("settings-search-dim-in-focus-mode"), description: msg("settings-search-inactive-blocks") },
  { tab: "appearance", label: msg("settings-search-load-local-file-images"), description: msg("settings-search-absolute-paths-permission-security") },
  { tab: "appearance", label: msg("settings-search-smooth-scrolling-experimental"), description: msg("settings-search-animated-journal-scrolling-webkit"), aliases: ["scroll animation"], level: "advanced" },
  { tab: "appearance", label: msg("settings-search-system-title-bar-window-controls"), description: msg("settings-search-native-frame-chrome") },
  { tab: "appearance", label: msg("settings-search-show-tine-in-the-system-tray"), description: msg("settings-search-tray-notification-area-menu-bar-icon") },
  { tab: "appearance", label: msg("settings-search-minimize-to-tray"), description: msg("settings-search-hide-taskbar-dock-minimize-system-tray") },
  { tab: "appearance", label: msg("settings-search-start-minimized-to-tray"), description: msg("settings-search-launch-hidden-startup-autostart-system-tray") },
  { tab: "editor", label: msg("settings-search-file-format"), description: msg("settings-search-new-pages-markdown-org") },
  { tab: "editor", label: msg("settings-search-logical-outdenting"), description: msg("settings-search-shift-tab-following-siblings-roam-config") },
  { tab: "editor", label: msg("settings-search-link-autocomplete-default"), description: msg("settings-search-og-adaptive-existing-typed-page-tag-completion"), level: "advanced" },
  { tab: "editor", label: msg("settings-search-switch-to-an-already-open-tab-when-navigating"), description: msg("settings-search-reuse-tabs"), level: "advanced" },
  { tab: "editor", label: msg("settings-search-learn-ctrl-k-choices"), description: msg("settings-search-adaptive-launcher-ranking-reset-history"), level: "advanced" },
  { tab: "editor", label: msg("settings-pinyin-label"), description: msg("settings-pinyin-description"), aliases: ["pinyin"], level: "advanced" },
  { tab: "editor", label: msg("settings-search-spell-checker"), description: msg("settings-search-dictionaries-languages-spelling") },
  { tab: "editor", label: msg("settings-search-copy-a-parent-block-s-sub-blocks"), description: msg("settings-search-clipboard-subtree"), level: "advanced" },
  { tab: "editor", label: msg("settings-search-strip-collapsed-when-copying"), description: msg("settings-search-clipboard-properties"), level: "advanced" },
  { tab: "editor", label: msg("settings-search-click-a-block-reference-to-zoom-in"), description: msg("settings-search-reference-navigation") },
  { tab: "journals", label: msg("settings-search-journal-date-format"), description: msg("settings-search-display-titles") },
  { tab: "journals", label: msg("settings-search-first-day-of-week"), description: msg("settings-search-calendar-monday-sunday") },
  { tab: "journals", label: msg("settings-search-carry-over"), description: msg("settings-search-buttons-context-header-last-days") },
  { tab: "journals", label: msg("settings-search-task-workflow"), description: msg("settings-search-todo-doing-now-later") },
  { tab: "journals", label: msg("settings-search-time-tracking"), description: msg("settings-search-logbook-clock") },
  { tab: "journals", label: msg("settings-search-new-journal-template"), description: msg("settings-search-default-journal-template") },
  { tab: "journals", label: msg("settings-search-quick-capture-enter-key"), description: msg("settings-search-capture-submit-new-block"), level: "advanced" },
  { tab: "journals", label: msg("settings-search-agenda-window"), description: msg("settings-search-scheduled-deadline-days") },
  { tab: "files", label: msg("settings-search-new-asset-filename"), description: msg("settings-search-paste-drag-media-names") },
  { tab: "files", label: msg("settings-search-watch-for-external-edits"), description: msg("settings-search-inotify-polling-network-filesystem") },
  { tab: "files", label: msg("settings-search-diagram-editors"), description: msg("settings-search-drawio-excalidraw-commands"), level: "advanced" },
  { tab: "backups", label: msg("settings-search-snapshots-to-keep"), description: msg("settings-search-recovery-retention-conflicts") },
  { tab: "backups", label: msg("settings-search-always-ask-before-applying-an-external-change"), description: msg("settings-search-external-edits-disk-reload-keep-mine-conflict") },
  { tab: "graph", label: msg("settings-search-graph"), description: msg("settings-search-folder-export-publish") },
  { tab: "graph", label: msg("settings-graph-folder-label"), description: msg("settings-graph-folder-search-description"), aliases: ["open graph folder"] },
  { tab: "graph", label: msg("settings-search-home-page"), description: msg("settings-search-home-start-startup-page") },
  { tab: "diagnostics", label: msg("settings-search-help-diagnostics"), description: msg("settings-search-diagnostic-report-crash-unclean-exit-save-slow") },
  { tab: "shortcuts", label: msg("settings-search-keyboard-shortcuts"), description: msg("settings-search-key-bindings-commands-remap") },
  { tab: "about", label: msg("settings-search-about"), description: msg("settings-search-version-licenses-updates") },
];

/** Match all query words against one entry's label, description and aliases.
 * O(entry text × query words), no I/O or failure. */
export function settingMatches(entry: SettingSearchEntry, query: string): boolean {
  const terms = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
  const haystack = [entry.label, entry.description, entry.keywords ?? "", ...(entry.aliases ?? [])]
    .flatMap((text) => typeof text === "string" ? [text] : [render(text), renderEnglish(text)]).join(" ").toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

/** Whether a tab has an advanced setting matching a nonempty query. Used to
 * reveal that section while searching. O(entries × query length), no I/O. */
export function advancedMatch(tab: Tab, query: string): boolean {
  return !!query.trim() && SETTING_SEARCH.some((entry) => entry.tab === tab && entry.level === "advanced" && settingMatches(entry, query));
}
