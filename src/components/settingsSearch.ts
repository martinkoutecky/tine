// Settings search vocabulary and matching contract: each visible setting has
// one label/description and optionally aliases; search matches all terms within
// that entry. Cost O(entries × query length), no I/O or failure.
import { KNOBS } from "../styleKnobs";
import { MEDIA_EDITORS } from "../mediaEditors";
import { msg, render, renderEnglish, type Msg } from "../i18n";
import type { SettingsTabId } from "../ui";
type Tab = SettingsTabId;

/** Mark visible query terms without changing matching or interpreting label HTML.
 * O(text length × query terms); overlapping matches form one highlighted span. */
export function settingHighlight(text: string, query: string): { text: string; marked: boolean }[] {
  const marked = new Uint8Array(text.length);
  const folded = text.toLowerCase();
  // Some Latin letters expand when lowercased (İ → i + combining dot).
  // Keep folded offsets attached to their original visible characters.
  const starts: number[] = [];
  const ends: number[] = [];
  let offset = 0;
  for (const char of text) {
    for (let i = 0; i < char.toLowerCase().length; i++) {
      starts.push(offset);
      ends.push(offset + char.length);
    }
    offset += char.length;
  }
  for (const term of query.toLowerCase().trim().split(/\s+/).filter(Boolean)) {
    let start = folded.indexOf(term);
    while (start >= 0) {
      marked.fill(1, starts[start], ends[start + term.length - 1]);
      start = folded.indexOf(term, start + 1);
    }
  }
  const parts: { text: string; marked: boolean }[] = [];
  for (let start = 0, end = 0; start < text.length; start = end) {
    end = start + 1;
    while (end < text.length && marked[end] === marked[start]) end++;
    parts.push({ text: text.slice(start, end), marked: !!marked[start] });
  }
  return parts;
}

export type SettingSearchEntry = { tab: Tab; label: string | Msg; description: string | Msg; keywords?: string | Msg; aliases?: string[]; level?: "advanced" };
export const SETTING_SEARCH: SettingSearchEntry[] = [
  { tab: "appearance", label: msg("settings-search-language"), description: msg("settings-search-interface-translation-locale-menus"), aliases: ["语言", "中文", "Chinese", "English"] },
  { tab: "appearance", label: msg("settings-search-theme-mode"), description: msg("settings-search-light-dark-system") },
  { tab: "appearance", label: msg("settings-search-style"), description: msg("settings-search-typography-journal-headings-presentation-theme") },
  { tab: "appearance", label: msg("settings-search-color-scheme"), description: msg("settings-search-default-nord-solarized-gruvbox-soft-medium-light") },
  { tab: "appearance", label: msg("themes-install-a-token-theme"), description: msg("themes-theme-packages-contain-only-whitelisted-color-tokens") },
  // Fine-tuning: one entry per knob in the table, so a new knob is searchable with no code here.
  ...KNOBS.filter((knob) => knob.type !== "heading").map((knob): SettingSearchEntry => ({
    tab: "appearance",
    label: knob.title,
    description: msg("settings-search-fine-tuning-style-knob-visual-detail"),
    keywords: knob.description,
    level: "advanced",
  })),
  { tab: "appearance", label: msg("settings-search-edit-custom-css"), description: msg("settings-search-logseq-custom-css-stylesheet-theming-tokens-css-variables"), level: "advanced" },
  { tab: "appearance", label: msg("css-edit-global"), description: msg("css-global-hint"), level: "advanced" },
  { tab: "appearance", label: msg("settings-search-disable-custom-css"), description: msg("settings-search-safe-mode-broken-stylesheet-unreadable-session"), level: "advanced" },
  { tab: "appearance", label: msg("settings-search-developer-tools"), description: msg("settings-search-inspector-inspect-element-styles-devtools"), level: "advanced" },
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
  { tab: "editor", label: msg("settings-search-your-name"), description: msg("settings-search-comment-author-name-margin-reply"), aliases: ["author", "identity"] },
  { tab: "editor", label: msg("settings-search-file-format"), description: msg("settings-search-new-pages-markdown-org") },
  { tab: "editor", label: msg("settings-search-logical-outdenting"), description: msg("settings-search-shift-tab-following-siblings-roam-config") },
  { tab: "editor", label: msg("settings-search-link-autocomplete-default"), description: msg("settings-search-og-adaptive-existing-typed-page-tag-completion"), level: "advanced" },
  { tab: "editor", label: msg("settings-search-switch-to-an-already-open-tab-when-navigating"), description: msg("settings-search-reuse-tabs"), level: "advanced" },
  { tab: "editor", label: msg("settings-search-learn-ctrl-k-choices"), description: msg("settings-search-adaptive-launcher-ranking-reset-history"), level: "advanced" },
  { tab: "editor", label: msg("settings-pinyin-label"), description: msg("settings-pinyin-description"), aliases: ["pinyin"], level: "advanced" },
  { tab: "editor", label: msg("settings-search-spell-checker"), description: msg("settings-search-dictionaries-languages-spelling") },
  { tab: "editor", label: msg("settings-ui-spellcheck-languages"), description: msg("settings-ui-tick-the-dictionaries-to-check-several-at-once-is") },
  { tab: "editor", label: msg("settings-search-copy-a-parent-block-s-sub-blocks"), description: msg("settings-search-clipboard-subtree"), level: "advanced" },
  { tab: "editor", label: msg("settings-search-strip-collapsed-when-copying"), description: msg("settings-search-clipboard-properties"), level: "advanced" },
  { tab: "editor", label: msg("settings-search-click-a-block-reference-to-zoom-in"), description: msg("settings-search-reference-navigation") },
  { tab: "journals", label: msg("settings-search-journal-date-format"), description: msg("settings-search-display-titles") },
  { tab: "journals", label: msg("settings-search-first-day-of-week"), description: msg("settings-search-calendar-monday-sunday") },
  { tab: "journals", label: msg("settings-search-carry-over"), description: msg("settings-search-buttons-context-header-last-days") },
  { tab: "journals", label: msg("settings-ui-show-carry-over-buttons"), description: msg("settings-ui-show-the-carry-buttons-next-to-journal-titles-off") },
  { tab: "journals", label: msg("settings-ui-carry-over-keeps-context"), description: msg("settings-ui-move-whole-blocks-that-contain-an-open-task-on") },
  { tab: "journals", label: msg("settings-ui-carry-over-header"), description: msg("settings-ui-add-a-carried-over-heading-above-carried-tasks") },
  { tab: "journals", label: msg("settings-ui-carry-last-n-days"), description: msg("settings-ui-n-for-the-carry-last-n-days-button-on") },
  { tab: "journals", label: msg("settings-search-task-workflow"), description: msg("settings-search-todo-doing-now-later") },
  { tab: "journals", label: msg("settings-search-time-tracking"), description: msg("settings-search-logbook-clock") },
  { tab: "journals", label: msg("settings-search-new-journal-template"), description: msg("settings-search-default-journal-template") },
  { tab: "journals", label: msg("settings-search-quick-capture-enter-key"), description: msg("settings-search-capture-submit-new-block"), level: "advanced" },
  { tab: "journals", label: msg("settings-search-agenda-window"), description: msg("settings-search-scheduled-deadline-days") },
  { tab: "files", label: msg("settings-search-new-asset-filename"), description: msg("settings-search-paste-drag-media-names") },
  { tab: "files", label: msg("settings-search-watch-for-external-edits"), description: msg("settings-search-inotify-polling-network-filesystem") },
  { tab: "files", label: msg("settings-search-diagram-editors"), description: msg("settings-search-drawio-excalidraw-commands"), aliases: MEDIA_EDITORS.map((editor) => editor.settingLabel), level: "advanced" },
  { tab: "files", label: msg("settings-orphaned-media"), description: msg("settings-ui-files-in-assets-that-no-block-links-to-deleting"), keywords: msg("settings-ui-permanently-delete-asset-files-in-logseq-tine-trash") },
  { tab: "backups", label: msg("settings-search-snapshots-to-keep"), description: msg("settings-search-recovery-retention-conflicts") },
  { tab: "backups", label: msg("settings-search-always-ask-before-applying-an-external-change"), description: msg("settings-search-external-edits-disk-reload-keep-mine-conflict") },
  { tab: "backups", label: msg("settings-ui-duplicate-journal-days"), description: msg("settings-ui-these-days-have-more-than-one-file-e-g") },
  { tab: "backups", label: msg("journal-filename-panel-journal-files-named-by-title"), description: msg("journal-filename-panel-these-journal-files-aren-t-named-after-their") },
  { tab: "backups", label: msg("settings-ui-conflicts"), description: msg("settings-ui-open-conflicts") },
  { tab: "graph", label: msg("settings-search-graph"), description: msg("settings-search-folder-export-publish") },
  { tab: "graph", label: msg("settings-graph-folder-label"), description: msg("settings-graph-folder-search-description"), aliases: ["open graph folder"] },
  { tab: "graph", label: msg("settings-search-home-page"), description: msg("settings-search-home-start-startup-page") },
  { tab: "graph", label: msg("graph-publish-publish"), description: msg("graph-publish-choose-a-destination-outside-the-graph-the-export"), keywords: msg("graph-publish-export-name") },
  { tab: "graph", label: msg("query-export-limit-setting-query-export-size-limit"), description: msg("query-export-limit-setting-default-1-gi-b-copied-assets-must-fit") },
  { tab: "plugins", label: msg("settings-ui-plugins"), description: msg("plugins-open-settings") },
  { tab: "diagnostics", label: msg("settings-search-help-diagnostics"), description: msg("settings-search-diagnostic-report-crash-unclean-exit-save-slow") },
  { tab: "diagnostics", label: msg("diagnostics-tab-rescan-graph"), description: msg("settings-prose-re-reads-every-file-in-the-open-graph-and-rebuilds") },
  { tab: "diagnostics", label: msg("error-toast-history-recent-error-messages"), description: msg("error-toast-history-the-last-20-red-error-messages-of-this") },
  { tab: "diagnostics", label: msg("graph-verification-verify-synchronized-graph"), description: msg("graph-verification-compare-the-exact-markdown-and-org-file-bytes") },
  { tab: "shortcuts", label: msg("settings-search-keyboard-shortcuts"), description: msg("settings-search-key-bindings-commands-remap") },
  { tab: "about", label: msg("settings-search-about"), description: msg("settings-search-version-licenses-updates") },
  { tab: "sharing", label: msg("quick-share-settings-account"), description: msg("settings-search-quick-share-account"), aliases: ["GitHub", "gist"] },
  { tab: "sharing", label: msg("quick-share-settings-shares"), description: msg("settings-search-quick-share-shares") },
  { tab: "sharing", label: msg("quick-share-settings-find"), description: msg("settings-search-quick-share-find") },
];

function textsOf(...parts: (string | Msg | undefined)[]): string {
  return parts.flatMap((text) => text === undefined ? [] : typeof text === "string" ? [text] : [render(text), renderEnglish(text)]).join(" ").toLowerCase();
}

/** A one-character term (any script) is too unspecific to match prose: a single
 * "发" occurs incidentally in many descriptions (GH #706). */
const isShortTerm = (term: string) => [...term].length === 1;
const queryTerms = (query: string) => query.toLowerCase().trim().split(/\s+/).filter(Boolean);

/** Whether every query word hits the entry's label, keywords or aliases. */
function nameMatches(entry: SettingSearchEntry, terms: string[]): boolean {
  const names = textsOf(entry.label, entry.keywords, ...(entry.aliases ?? []));
  return terms.every((term) => names.includes(term));
}

/** Match all query words against one entry's label, description, keywords and
 * aliases; a one-character word matches everything but the description.
 * O(entry text × query words), no I/O or failure. */
export function settingMatches(entry: SettingSearchEntry, query: string): boolean {
  const terms = queryTerms(query);
  const haystack = textsOf(entry.label, entry.description, entry.keywords, ...(entry.aliases ?? []));
  const names = textsOf(entry.label, entry.keywords, ...(entry.aliases ?? []));
  return terms.every((term) => (isShortTerm(term) ? names : haystack).includes(term));
}

/** Matching entries with label/keyword/alias matches before description-only
 * matches; the given order is kept within each group. O(entries × text × words). */
export function rankedSettingMatches(entries: readonly SettingSearchEntry[], query: string): SettingSearchEntry[] {
  const terms = queryTerms(query);
  const hits = entries.filter((entry) => settingMatches(entry, query));
  return [...hits.filter((entry) => nameMatches(entry, terms)), ...hits.filter((entry) => !nameMatches(entry, terms))];
}

/** Whether a tab has an advanced setting matching a nonempty query. Used to
 * reveal that section while searching. O(entries × query length), no I/O. */
export function advancedMatch(tab: Tab, query: string): boolean {
  return !!query.trim() && SETTING_SEARCH.some((entry) => entry.tab === tab && entry.level === "advanced" && settingMatches(entry, query));
}
