# Settings inventory

Generated from the descriptors consumed by the Settings controls and search.
Run `node scripts/generate-settings-inventory.mjs` after changing a descriptor;
`node scripts/generate-settings-inventory.mjs --check` checks staleness.

IDs are stable navigation targets (`data-setting-id`); labels resolve in the selected language.
Search also accepts English terms of two or more characters and declared aliases. A single character matches only displayed-language text.

The inventory includes gated controls (CM6, desktop, touch, Sharing) even where a build or device hides them.

<!-- tab:appearance -->

## appearance

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `settings-comments-display` | `settings-comments-display` | Comments | Basic |
| `settings-author-marks` | `settings-author-marks` | Show/Hide author marks | Basic |
| `settings-show-workspaces` | `settings-show-workspaces` | Show Workspaces | Advanced |
| `settings-show-help-button` | `settings-show-help-button` | Show the help button | Advanced |
| `settings-ui-interface-size` | `settings-ui-interface-size` | Interface size | Basic |
| `settings-mobile-navigation-label` | `settings-mobile-navigation-label` | Navigation buttons at the bottom | Basic |
| `settings-ui-wide-mode` | `settings-ui-wide-mode` | Wide mode | Basic |
| `settings-ui-document-mode` | `settings-ui-document-mode` | Document mode | Basic |
| `settings-ui-document-mode-enter-creates-a-new-block` | `settings-ui-document-mode-enter-creates-a-new-block` | Document-mode Enter creates a new block | Basic |
| `settings-ui-comment-margin` | `settings-ui-comment-margin` | Show/Hide comments | Basic |
| `settings-ui-show-brackets` | `settings-ui-show-brackets` | Show brackets | Basic |
| `settings-ui-typographic-replacements` | `settings-ui-typographic-replacements` | Typographic replacements | Basic |
| `settings-ui-wrap-code-lines` | `settings-ui-wrap-code-lines` | Wrap code lines | Basic |
| `setting-sticky-parents` | `setting-sticky-parents` | Sticky parent bullets | Basic |
| `settings-ui-auto-pair-brackets-quotes` | `settings-ui-auto-pair-brackets-quotes` | Auto-pair brackets & quotes | Basic |
| `settings-ui-space-after-inserting-a-reference` | `settings-ui-space-after-inserting-a-reference` | Space after inserting a reference | Basic |
| `settings-ui-dim-in-focus-mode` | `settings-ui-dim-in-focus-mode` | Dim in focus mode | Basic |
| `settings-ui-load-local-file-images` | `settings-ui-load-local-file-images` | Load local-file images | Basic |
| `settings-ui-right-to-left-note-layout` | `settings-ui-right-to-left-note-layout` | Right-to-left note layout | Advanced |
| `settings-ui-smooth-scrolling-experimental` | `settings-ui-smooth-scrolling-experimental` | Smooth scrolling (experimental) | Advanced |
| `settings-ui-system-title-bar-window-controls` | `settings-ui-system-title-bar-window-controls` | System title bar & window controls | Basic |
| `settings-ui-window-controls` | `settings-ui-window-controls` | Window controls | Basic |
| `content-width-fields-standard-page-width` | `content-width-fields-standard-page-width` | Standard page width | Advanced |
| `content-width-fields-wide-page-width` | `content-width-fields-wide-page-width` | Wide page width | Advanced |
| `css-edit-graph` | `css-edit-graph` | Edit custom.css | Advanced |
| `css-edit-global` | `css-edit-global` | Edit global CSS | Advanced |
| `settings-search-disable-custom-css` | `settings-search-disable-custom-css` | Disable custom CSS | Advanced |
| `settings-search-developer-tools` | `settings-search-developer-tools` | Developer tools | Advanced |
| `settings-prose-show-tine-in-the-system-tray` | `settings-prose-show-tine-in-the-system-tray` | Show Tine in the system tray | Basic |
| `settings-prose-minimize-to-tray` | `settings-prose-minimize-to-tray` | Minimize to tray | Basic |
| `settings-prose-start-minimized-to-tray` | `settings-prose-start-minimized-to-tray` | Start minimized to tray | Basic |
| `settings-ui-mode` | `settings-ui-mode` | Mode | Basic |
| `settings-ui-accent-color` | `settings-ui-accent-color` | Accent color | Basic |
| `settings-language-label` | `settings-language-label` | Language | Basic |
| `knob.embed-breadcrumbs` | `settings-embed-breadcrumbs` | Block embed breadcrumbs | Advanced |
| `knob.embed-shading` | `settings-embedded-blocks` | Embedded blocks | Advanced |
| `knob.guide-colors` | `settings-color-indent-guides-by-level` | Color indent guides by level | Advanced |
| `knob.external-link-underline` | `settings-links-to-the-web-or-a-file` | Links to the web or a file | Advanced |
| `knob.missing-page-color` | `settings-links-to-pages-that-do-not-exist` | Links to pages that do not exist yet | Advanced |

<!-- tab:themes -->

## themes

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `theme.gallery.default` | `themes-default` | Default | Basic |
| `themes-style` | `themes-style` | Style | Basic |
| `themes-typography` | `themes-typography` | Typography | Basic |
| `themes-color-scheme` | `themes-color-scheme` | Color scheme | Basic |
| `themes-install-a-token-theme` | `themes-install-a-token-theme` | Install a token theme | Basic |
| `theme.gallery.nord` | `Nord` | Nord | Basic |
| `theme.gallery.solarized` | `Solarized` | Solarized | Basic |
| `theme.gallery.gruvbox` | `Gruvbox` | Gruvbox | Basic |
| `theme.gallery.soft` | `Soft` | Soft | Basic |

<!-- tab:journals -->

## journals

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `settings-ui-new-journal-template` | `settings-ui-new-journal-template` | New-journal template | Basic |
| `settings-ui-journal-date-format` | `settings-ui-journal-date-format` | Journal date format | Basic |
| `settings-ui-first-day-of-week` | `settings-ui-first-day-of-week` | First day of week | Basic |
| `settings-ui-show-carry-over-buttons` | `settings-ui-show-carry-over-buttons` | Show carry-over buttons | Basic |
| `settings-ui-carry-over-keeps-context` | `settings-ui-carry-over-keeps-context` | Carry-over keeps context | Basic |
| `settings-ui-carry-over-header` | `settings-ui-carry-over-header` | Carry-over header | Basic |
| `settings-ui-carry-last-n-days` | `settings-ui-carry-last-n-days` | Carry “last N days” | Basic |
| `settings-ui-task-workflow` | `settings-ui-task-workflow` | Task workflow | Basic |
| `settings-ui-time-tracking` | `settings-ui-time-tracking` | Time tracking | Basic |
| `settings-ui-quick-capture-enter-key` | `settings-ui-quick-capture-enter-key` | Quick-capture Enter key | Advanced |
| `settings-ui-agenda-window` | `settings-ui-agenda-window` | Agenda window | Basic |

<!-- tab:editor -->

## editor

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `settings-ui-file-format` | `settings-ui-file-format` | File format | Basic |
| `settings-auto-embed-video-links` | `settings-auto-embed-video-links` | Auto-embed pasted video links | Basic |
| `settings-ui-spell-checker` | `settings-ui-spell-checker` | Spell checker | Basic |
| `settings-ui-logical-outdenting` | `settings-ui-logical-outdenting` | Logical outdenting | Basic |
| `settings-ui-spellcheck-languages` | `settings-ui-spellcheck-languages` | Spellcheck languages | Basic |
| `settings-reference-previews` | `settings-reference-previews` | Reference previews | Advanced |
| `settings-hover-preview-delay` | `settings-hover-preview-delay` | Hover preview delay | Advanced |
| `settings-cm6-label` | `settings-cm6-label` | Experimental: CodeMirror editor | Advanced |
| `settings-ui-link-autocomplete-default` | `settings-ui-link-autocomplete-default` | Link autocomplete default | Advanced |
| `settings-ui-switch-to-an-already-open-tab-when-navigating` | `settings-ui-switch-to-an-already-open-tab-when-navigating` | Switch to an already-open tab when navigating | Advanced |
| `settings-result-destination-label` | `settings-result-destination-label` | Search results open in | Advanced |
| `settings-pinyin-label` | `settings-pinyin-label` | Match Chinese page names by Pinyin | Advanced |
| `settings-ui-learn-ctrl-k-choices` | `settings-ui-learn-ctrl-k-choices` | Learn Ctrl+K choices | Advanced |
| `settings-ui-strip-collapsed-when-copying` | `settings-ui-strip-collapsed-when-copying` | Strip collapsed:: when copying | Advanced |
| `settings-ui-click-a-block-reference-to-zoom-in` | `settings-ui-click-a-block-reference-to-zoom-in` | Click a block reference to zoom in | Basic |
| `links-remembered-schemes` | `links-remembered-schemes` | Remembered app links | Basic |
| `comment-name-label` | `comment-name-label` | Your name | Basic |

<!-- tab:backups -->

## backups

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `settings-ui-snapshots-to-keep` | `settings-ui-snapshots-to-keep` | Snapshots to keep | Basic |
| `settings-prose-always-ask-before-applying-an-external-change` | `settings-prose-always-ask-before-applying-an-external-change` | Always ask before applying an external change | Basic |
| `settings-ui-duplicate-journal-days` | `settings-ui-duplicate-journal-days` | Duplicate journal days | Basic |
| `settings-ui-conflicts` | `settings-ui-conflicts` | Conflicts | Basic |
| `settings-available-snapshots` | `settings-available-snapshots` | Available snapshots | Basic |
| `journal-filename-panel-journal-files-named-by-title` | `journal-filename-panel-journal-files-named-by-title` | Journal files named by title | Basic |

<!-- tab:files -->

## files

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `settings-ui-new-asset-filename` | `settings-ui-new-asset-filename` | New asset filename | Basic |
| `settings-ui-watch-for-external-edits` | `settings-ui-watch-for-external-edits` | Watch for external edits | Basic |
| `settings-ui-diagram-editors` | `settings-ui-diagram-editors` | Diagram editors | Advanced |
| `settings-orphaned-media` | `settings-orphaned-media` | Orphaned media | Basic |
| `media-editor.drawio` | `draw.io command` | draw.io command | Advanced |
| `media-editor.excalidraw` | `Excalidraw command` | Excalidraw command | Advanced |

<!-- tab:graph -->

## graph

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `git-settings-label` | `git-settings-label` | Commit changes to git | Basic |
| `seen-settings-label` | `seen-settings-label` | Changed since you last looked | Basic |
| `settings-graph-folder-label` | `settings-graph-folder-label` | Graph folder | Basic |
| `home-page-setting-home-page` | `home-page-setting-home-page` | Home page | Basic |
| `graph-publish-graph` | `graph-publish-graph` | Graph | Basic |
| `graph-publish-publish` | `graph-publish-publish` | Publish | Basic |
| `query-export-limit-setting-query-export-size-limit` | `query-export-limit-setting-query-export-size-limit` | Query export size limit | Basic |

<!-- tab:sharing -->

## sharing

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `quick-share-settings-account` | `quick-share-settings-account` | GitHub account | Basic |
| `quick-share-settings-shares` | `quick-share-settings-shares` | Share links on this device | Basic |
| `quick-share-settings-markers` | `quick-share-settings-markers` | Pending in this graph | Basic |
| `quick-share-settings-find` | `quick-share-settings-find` | Find shares on GitHub | Basic |

<!-- tab:diagnostics -->

## diagnostics

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `diagnostics-tab-help-diagnostics` | `diagnostics-tab-help-diagnostics` | Help & diagnostics | Basic |
| `diagnostics-refresh-everything` | `diagnostics-refresh-everything` | Refresh everything on every save (debug) | Basic |
| `diagnostics-tab-rescan-graph` | `diagnostics-tab-rescan-graph` | Rescan graph | Basic |
| `error-toast-history-recent-error-messages` | `error-toast-history-recent-error-messages` | Recent error messages | Basic |
| `graph-verification-verify-synchronized-graph` | `graph-verification-verify-synchronized-graph` | Verify synchronized graph | Basic |

<!-- tab:about -->

## about

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `about-check-for-updates-automatically` | `about-check-for-updates-automatically` | Check for updates automatically | Basic |
| `settings-ui-about` | `settings-ui-about` | About | Basic |

<!-- tab:plugins -->

## plugins

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `plugins-install-a-local-package` | `plugins-install-a-local-package` | Install a local package | Basic |
| `settings-ui-plugins` | `settings-ui-plugins` | Plugins | Basic |

<!-- tab:shortcuts -->

## shortcuts

| Setting ID | Catalog key / package label | Label | Level |
|---|---|---|---|
| `settings-ui-keyboard-shortcuts` | `settings-ui-keyboard-shortcuts` | Keyboard shortcuts | Basic |

## Device and registry entries

In addition to the rows above, search and Themes use the same installed theme and signed community theme registries: each package contributes its name and description, with a distinct ID for its row. Built-in gallery cards use the gallery registry.

Installed plugins contribute each manifest setting's label and description, plus the plugin name. Choosing a result opens that exact plugin version's settings form. Plugin and theme text is supplied by its author and stays verbatim in every interface language.
