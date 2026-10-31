# Core settings disclosure inventory

Reviewed 2026-10-31 (arrival audit L4-04). Each row records the stable search
label key, visible English label, and current disclosure level. Basic settings
stay visible; Advanced controls are revealed by their section or by search.
Conditional mobile, CM6, Sharing and Plugins controls are included even when
unavailable in the current build. Fine-tuning rows come from the knob registry.

The guard in `src/components/settingsInventory.test.ts` checks all search keys,
levels and tab IDs. Add the inventory row when adding a setting. Plugin-owned
settings are declared by each installed manifest and rendered in Plugins; their
dynamic keys are outside this core inventory.

<!-- tab:appearance -->

## Appearance

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `appearance` | `settings-search-language` | Language | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-theme-mode` | Theme mode | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-style` | Style | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-color-scheme` | Color scheme | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `themes-install-a-token-theme` | Install a token theme | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-edit-custom-css` | Edit custom.css | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `appearance` | `css-edit-global` | Edit global CSS | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `appearance` | `settings-search-disable-custom-css` | Disable custom CSS | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `appearance` | `settings-search-developer-tools` | Developer tools | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `appearance` | `settings-search-accent-color` | Accent color | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-interface-size` | Interface size | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-wide-mode` | Wide mode | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-standard-page-width` | Standard page width | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `appearance` | `settings-search-wide-page-width` | Wide page width | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `appearance` | `settings-search-document-mode` | Document mode | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-document-mode-enter-creates-a-new-block` | Document-mode Enter creates a new block | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-show-brackets` | Show brackets | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-typographic-replacements` | Typographic replacements | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-wrap-code-lines` | Wrap code lines | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `setting-sticky-parents` | Sticky parent bullets | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-auto-pair-brackets-quotes` | Auto-pair brackets & quotes | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-space-after-inserting-a-reference` | Space after inserting a reference | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-dim-in-focus-mode` | Dim in focus mode | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-load-local-file-images` | Load local-file images | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-mobile-navigation-label` | Navigation buttons at the bottom | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-smooth-scrolling-experimental` | Smooth scrolling (experimental) | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `appearance` | `settings-search-system-title-bar-window-controls` | System title bar & window controls | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-show-tine-in-the-system-tray` | Show Tine in the system tray | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-minimize-to-tray` | Minimize to tray | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-search-start-minimized-to-tray` | Start minimized to tray | Basic | Visible workflow, accessibility, permission or safety choice. |
| `appearance` | `settings-embed-breadcrumbs` | Block embed breadcrumbs | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `appearance` | `settings-embedded-blocks` | Embedded blocks | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `appearance` | `settings-links-to-the-web-or-a-file` | Links to the web or a file | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `appearance` | `settings-links-to-pages-that-do-not-exist` | Links to pages that do not exist yet | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |

<!-- tab:editor -->

## Editor

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `editor` | `settings-search-your-name` | Your name | Basic | Visible workflow, accessibility, permission or safety choice. |
| `editor` | `settings-cm6-label` | Experimental: CodeMirror editor | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `editor` | `settings-search-file-format` | File format | Basic | Visible workflow, accessibility, permission or safety choice. |
| `editor` | `settings-search-logical-outdenting` | Logical outdenting | Basic | Visible workflow, accessibility, permission or safety choice. |
| `editor` | `settings-search-link-autocomplete-default` | Link autocomplete default | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `editor` | `settings-search-switch-to-an-already-open-tab-when-navigating` | Switch to an already-open tab when navigating | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `editor` | `settings-search-learn-ctrl-k-choices` | Learn Ctrl+K choices | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `editor` | `settings-pinyin-label` | Match Chinese page names by Pinyin | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `editor` | `settings-search-spell-checker` | Spell checker | Basic | Visible workflow, accessibility, permission or safety choice. |
| `editor` | `settings-ui-spellcheck-languages` | Spellcheck languages | Basic | Visible workflow, accessibility, permission or safety choice. |
| `editor` | `settings-search-copy-a-parent-block-s-sub-blocks` | Copy a parent block's sub-blocks | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `editor` | `settings-search-strip-collapsed-when-copying` | Strip collapsed:: when copying | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `editor` | `settings-search-click-a-block-reference-to-zoom-in` | Click a block reference to zoom in | Basic | Visible workflow, accessibility, permission or safety choice. |
| `editor` | `settings-result-destination-label` | Search results open in | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |

<!-- tab:journals -->

## Journals

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `journals` | `settings-search-journal-date-format` | Journal date format | Basic | Visible workflow, accessibility, permission or safety choice. |
| `journals` | `settings-search-first-day-of-week` | First day of week | Basic | Visible workflow, accessibility, permission or safety choice. |
| `journals` | `settings-search-carry-over` | Carry-over | Basic | Visible workflow, accessibility, permission or safety choice. |
| `journals` | `settings-ui-show-carry-over-buttons` | Show carry-over buttons | Basic | Visible workflow, accessibility, permission or safety choice. |
| `journals` | `settings-ui-carry-over-keeps-context` | Carry-over keeps context | Basic | Visible workflow, accessibility, permission or safety choice. |
| `journals` | `settings-ui-carry-over-header` | Carry-over header | Basic | Visible workflow, accessibility, permission or safety choice. |
| `journals` | `settings-ui-carry-last-n-days` | Carry “last N days” | Basic | Visible workflow, accessibility, permission or safety choice. |
| `journals` | `settings-search-task-workflow` | Task workflow | Basic | Visible workflow, accessibility, permission or safety choice. |
| `journals` | `settings-search-time-tracking` | Time tracking | Basic | Visible workflow, accessibility, permission or safety choice. |
| `journals` | `settings-search-new-journal-template` | New-journal template | Basic | Visible workflow, accessibility, permission or safety choice. |
| `journals` | `settings-search-quick-capture-enter-key` | Quick-capture Enter key | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `journals` | `settings-search-agenda-window` | Agenda window | Basic | Visible workflow, accessibility, permission or safety choice. |

<!-- tab:files -->

## Files

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `files` | `settings-search-new-asset-filename` | New asset filename | Basic | Visible workflow, accessibility, permission or safety choice. |
| `files` | `settings-search-watch-for-external-edits` | Watch for external edits | Basic | Visible workflow, accessibility, permission or safety choice. |
| `files` | `settings-search-diagram-editors` | Diagram editors | Advanced | Optional fine-tuning, compatibility or troubleshooting choice. |
| `files` | `settings-orphaned-media` | Orphaned media | Basic | Visible workflow, accessibility, permission or safety choice. |

<!-- tab:backups -->

## Backups & recovery

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `backups` | `settings-search-snapshots-to-keep` | Snapshots to keep | Basic | Visible workflow, accessibility, permission or safety choice. |
| `backups` | `settings-search-always-ask-before-applying-an-external-change` | Always ask before applying an external change | Basic | Visible workflow, accessibility, permission or safety choice. |
| `backups` | `settings-ui-duplicate-journal-days` | Duplicate journal days | Basic | Visible workflow, accessibility, permission or safety choice. |
| `backups` | `journal-filename-panel-journal-files-named-by-title` | Journal files named by title | Basic | Visible workflow, accessibility, permission or safety choice. |
| `backups` | `settings-ui-conflicts` | Conflicts | Basic | Visible workflow, accessibility, permission or safety choice. |

<!-- tab:graph -->

## Graph

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `graph` | `settings-search-graph` | Graph | Basic | Visible workflow, accessibility, permission or safety choice. |
| `graph` | `settings-graph-folder-label` | Graph folder | Basic | Visible workflow, accessibility, permission or safety choice. |
| `graph` | `settings-search-home-page` | Home page | Basic | Visible workflow, accessibility, permission or safety choice. |
| `graph` | `graph-publish-publish` | Publish | Basic | Visible workflow, accessibility, permission or safety choice. |
| `graph` | `git-settings-label` | Commit changes to git | Basic | Visible workflow, accessibility, permission or safety choice. |
| `graph` | `seen-settings-label` | Changed since you last looked | Basic | Visible workflow, accessibility, permission or safety choice. |
| `graph` | `query-export-limit-setting-query-export-size-limit` | Query export size limit | Basic | Visible workflow, accessibility, permission or safety choice. |

<!-- tab:sharing -->

## Sharing

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `sharing` | `quick-share-settings-account` | GitHub account | Basic | Visible workflow, accessibility, permission or safety choice. |
| `sharing` | `quick-share-settings-shares` | Share links on this device | Basic | Visible workflow, accessibility, permission or safety choice. |
| `sharing` | `quick-share-settings-find` | Find shares on GitHub | Basic | Visible workflow, accessibility, permission or safety choice. |

<!-- tab:plugins -->

## Plugins

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `plugins` | `settings-ui-plugins` | Plugins | Basic | Visible workflow, accessibility, permission or safety choice. |

<!-- tab:diagnostics -->

## Help & diagnostics

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `diagnostics` | `settings-search-help-diagnostics` | Help & diagnostics | Basic | Visible workflow, accessibility, permission or safety choice. |
| `diagnostics` | `diagnostics-tab-rescan-graph` | Rescan graph | Basic | Visible workflow, accessibility, permission or safety choice. |
| `diagnostics` | `error-toast-history-recent-error-messages` | Recent error messages | Basic | Visible workflow, accessibility, permission or safety choice. |
| `diagnostics` | `graph-verification-verify-synchronized-graph` | Verify synchronized graph | Basic | Visible workflow, accessibility, permission or safety choice. |

<!-- tab:shortcuts -->

## Keyboard shortcuts

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `shortcuts` | `settings-search-keyboard-shortcuts` | Keyboard shortcuts | Basic | Visible workflow, accessibility, permission or safety choice. |

<!-- tab:about -->

## About

| Tab ID | Setting key | Label | Level | Rationale |
|---|---|---|---|---|
| `about` | `settings-search-about` | About | Basic | Visible workflow, accessibility, permission or safety choice. |
