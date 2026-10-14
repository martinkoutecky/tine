## Settings › Editor › Advanced
settings-pinyin-label = Match Chinese page names by Pinyin
settings-pinyin-description = Find Han titles and aliases with Mandarin Pinyin or syllable initials. Native name matches come first. This preference stays on this device.
settings-pinyin-reset = Reset Pinyin matching to language default
toast-pinyin-save-failed = Could not save the Pinyin matching preference.
toast-pinyin-load-failed = Could not load the Pinyin matching preference.

## Settings › Appearance
settings-language-label = Language
settings-language-description = The language of { -brand }'s menus, settings and messages. Your notes are never translated.
# $language (string): the endonym of the language the system default resolves to.
settings-language-system = System default ({ $language })
toast-language-load-failed = Could not load the language preference; using the system language.
toast-language-save-failed = Could not save the language preference.

## Settings › About
settings-about-store-updates = Updates come from the Microsoft Store.

## Settings › Graph
settings-graph-folder-label = Graph folder
settings-graph-folder-reveal = Show in folder
settings-graph-folder-search-description = show in folder reveal file manager Finder Explorer location path where

## Graph creation
graph-wsl-rootfs-refused = Cannot create a graph here: Windows access to a WSL1 rootfs can corrupt Linux files. Choose \\wsl.localhost\<distro>\… instead.
setting-sticky-parents = Sticky parent bullets
setting-sticky-parents-hint = Keep offscreen ancestors at the top of scrolling outlines. Click one to return to it. On by default; saved on this device.
sticky-parents-load-error = Could not load sticky parent bullets
sticky-parents-save-error = Could not remember sticky parent bullets
settings-embed-breadcrumbs = Block embed breadcrumbs
settings-embed-breadcrumbs-hint = Show the source page and parent trail above embedded blocks. Off by default.

# Interface text; variables carry untranslated names, paths or diagnostic details.
# Key names are literal shortcut notation and must retain their spelling.
# $platform is mac or other. Keep the literal key names inside <kbd> tags.
settings-inspector-hint = { $platform ->
    [mac] Opens the inspector to see which rules style an element (also <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>J</kbd>).
   *[other] Opens the inspector to see which rules style an element (also <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>J</kbd>).
    }
settings-theme-light = Light
settings-theme-system = System
settings-theme-dark = Dark
settings-could-not-load-asset-filename-format = Could not load asset filename format.
settings-custom-css = Custom CSS

settings-open-developer-tools = Open developer tools
settings-opacity = { $title } opacity
settings-fine-tuning = Fine-tuning
settings-small-visual-details-saved-on-this-device = Small visual details, saved on this device. Each starts at Tine's default look; your <code>logseq/custom.css</code> still takes priority, and Disable custom CSS below switches these off for the session too.
settings-reset = Reset
settings-reset-control = Reset { $title }
settings-appearance = Appearance
settings-could-not-read-the-system-tray-state = Could not read the system tray state.
settings-could-not-load-copy-subtree-preference = Could not load copy subtree preference.
settings-could-not-load-collapsed-copy-preference = Could not load collapsed copy preference.
settings-could-not-load-block-reference-preference = Could not load block reference preference.
settings-newest-first = Newest first
settings-most-recent-first-journal-pages-by = Most recent first — journal pages by their date, others by when the file was last modified
settings-oldest-first = Oldest first
settings-oldest-first-journal-pages-by-their = Oldest first — journal pages by their date, others by file modified time
settings-priority-a-c = Priority A→C
settings-highest-priority-a-first-unprioritized-last = Highest priority ([#A]) first; unprioritized last
settings-page-a-z = Page A→Z
settings-alphabetically-by-the-page-each-result-lives = Alphabetically by the page each result lives on
settings-deadline = Deadline
settings-soonest-deadline-first-blocks-without-a-deadline = Soonest DEADLINE first; blocks without a deadline last
settings-scheduled = Scheduled
settings-soonest-scheduled-first-blocks-without-one-last = Soonest SCHEDULED first; blocks without one last
settings-could-not-load-local-image-access-preference = Could not load local image access preference.
settings-could-not-load-editor-command = Could not load { $id } editor command.
settings-could-not-load-tab-reuse-preference = Could not load tab reuse preference.
settings-could-not-load-reference-completion-preference = Could not load reference completion preference.
# $error is an untranslated diagnostic from the operating system or backend.
settings-could-not-remember-settings-size = Could not remember Settings size: { $error }
# $error is an untranslated diagnostic from the operating system or backend.
settings-could-not-load-settings-size = Could not load Settings size: { $error }
settings-could-not-apply-spellcheck-settings = Could not apply spellcheck settings.
settings-could-not-load-spellcheck-dictionaries = Could not load spellcheck dictionaries.
settings-could-not-load-spellcheck-preference = Could not load spellcheck preference.
settings-could-not-load-spellcheck-languages = Could not load spellcheck languages.
settings-embeds = Embeds
settings-embedded-blocks = Embedded blocks
settings-shade-embedded-blocks-like-logseq-or-show = Shade embedded blocks like Logseq, or show only the accent line and dot.
settings-shaded-box = Shaded box
settings-accent-line-and-dot-only = Accent line and dot only
settings-links = Links
settings-links-to-the-web-or-a-file = Links to the web or a file
settings-the-line-under-a-link-that-leaves = The line under a link that leaves the graph. Page links never have one, so the two kinds read differently.
settings-solid-underline = Solid underline
settings-dotted-underline = Dotted underline
settings-no-underline = No underline
settings-links-to-pages-that-do-not-exist = Links to pages that do not exist yet
settings-how-strongly-the-dotted-link-to-a = How strongly the dotted link to a page you have not created stands out.
settings-dimmed-link-color = Dimmed link color
settings-stronger = Stronger
settings-same-as-other-links = Same as other links
settings-could-not-load-style-settings = Could not load style settings.
settings-could-not-load-automatic-update-preference = Could not load automatic update preference.
content-width-fields-standard-page-width = Standard page width

content-width-fields-maximum-reading-column-width-on-this-device-reset = Maximum reading-column width on this device. Reset uses the active theme's default (810 px in Tine's built-in themes).

content-width-fields-standard-page-width-in-pixels = Standard page width in pixels

content-width-fields-px = px

content-width-fields-reset = Reset

content-width-fields-wide-page-width = Wide page width

content-width-fields-wide-mode-can-fill-the-available-pane-or = Wide mode can fill the available pane or stop at a custom maximum. Saved on this device.

content-width-fields-wide-page-width-mode = Wide page width mode

content-width-fields-fill-pane = Fill pane

content-width-fields-custom-maximum = Custom maximum

content-width-fields-wide-page-width-in-pixels = Wide page width in pixels

diagnostics-tab-help-diagnostics = Help & diagnostics

diagnostics-tab-tine-keeps-a-small-bounded-flight-recorder-of = Tine keeps a small, bounded flight recorder of this run and the previous one, including whether the previous run closed cleanly. It records operation names, outcomes, timings, counts, platform and build information.

diagnostics-tab-it-does-not-record-graph-content-file-paths = It does not record graph content, file paths, page titles, queries, URLs, credentials, or the detailed opt-in debug log. It is kept in Tine's private app data (at most 1 MiB), never in your graph, and nothing is uploaded. You choose whether to copy or save a report and share it.

diagnostics-tab-a-report-also-carries-statistics-about-your-graph = A report also carries statistics about your graph, such as how long each step of opening it took and the sizes of its pages, as numbers only. They never include a page name, any text, or a hash of either.

diagnostics-tab-creating = Creating…

diagnostics-tab-create-diagnostic-report = Create diagnostic report

diagnostics-tab-copy-report = Copy report

diagnostics-tab-save-report = Save report…

diagnostics-tab-clear-recorded-events = Clear recorded events

diagnostics-tab-rescanning = Rescanning…

diagnostics-tab-rescan-graph = Rescan graph

diagnostics-tab-large-report-this-preview-is-shortened-to-keep = Large report: this preview is shortened to keep Settings responsive. Copy report exports the complete report.

diagnostics-tab-help-improve-tine-s-parser = Help improve Tine's parser

error-toast-history-recent-error-messages = Recent error messages

error-toast-history-the-last-20-red-error-messages-of-this = The last 20 red error messages of this session, newest first. They are kept in memory only, are not part of the diagnostic report above (they can name pages), and are gone when Tine closes.

error-toast-history-no-error-messages-this-session = No error messages this session.

error-toast-history-copy = Copy

graph-verification-verify-synchronized-graph = Verify synchronized graph

graph-verification-compare-the-exact-markdown-and-org-file-bytes = Compare the exact Markdown and Org file bytes on two devices. The report includes file paths and page names, but not file contents. Nothing is uploaded automatically.

graph-verification-verifying = Verifying…

graph-verification-create-graph-verification-report = Create graph verification report

graph-verification-cancel = Cancel

graph-verification-copy-graph-report = Copy graph report

graph-verification-save-graph-report = Save graph report…

graph-verification-reading-graph-file-list = Reading graph file list…

graph-verification-files = { $processed } / { $total } files

graph-verification-report-from-the-other-device = Report from the other device

graph-verification-paste-the-graph-verification-report-here = Paste the graph verification report here

graph-verification-compare-reports = Compare reports

graph-verification-the-source-file-sets-and-bytes-match = The source file sets and bytes match.

graph-verification-at-least-one-report-is-incomplete-no-match = At least one report is incomplete. No match can be confirmed.

home-page-setting-home-page = Home page

home-page-setting-search-home-pages = Search home pages

home-page-setting-search-pages = Search pages…

home-page-setting-cancel = Cancel

home-page-setting-change = Change…

home-page-setting-clear = Clear

home-page-setting-open-this-page-when-the-graph-opens-only = Open this page when the graph opens. Only existing pages can be selected.

diagnostics-tab-preview-file = Report preview · { $file }
graph-verification-preview-file = Report preview · { $file }
graph-verification-summary =
    { $complete ->
        [1] Complete · { $files } files · { $bytes } bytes
       *[other] Incomplete · { $files } files · { $bytes } bytes
    }
