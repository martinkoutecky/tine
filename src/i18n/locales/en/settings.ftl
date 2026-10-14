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
