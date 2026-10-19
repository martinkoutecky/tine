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
settings-mobile-navigation-label = Navigation buttons at the bottom
settings-mobile-navigation-hint = Phones and tablets: put Sidebar, Search, Back and Forward in a row at the bottom of the screen, within thumb reach, instead of the top bar. The row gives way to the editing toolbar while the keyboard is up. Off by default; saved on this device.
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

# Remaining interface text (GH #107). Inline code and syntax stay unchanged.
settings-search-result-location =
    { $level ->
        [advanced] { $tab } › Advanced
       *[other] { $tab }
    }
settings-journal-template-missing =
    { $state ->
        [missing] { $name } (not found)
       *[unavailable] { $name } (unavailable)
    }
# The config key and filename are Logseq syntax, not translated.
settings-agenda-days-ahead =
    { $source ->
        [graph] days ahead · set by :scheduled/future-days in config.edn
       *[device] days ahead
    }
settings-available-snapshots = Available snapshots
settings-backup-load-failed = Couldn't load backup settings: { $error }
settings-journal-file-merged = Merged { $name } into { $canonical }
settings-journal-file-renamed = Renamed { $name } → { $next }
settings-detecting = Detecting…
settings-autodetect = Autodetect
settings-orphaned-media = Orphaned media
settings-scanning = Scanning…
settings-rescan = Rescan
settings-scan-for-orphans = Scan for orphans
settings-ui-appearance = Appearance
settings-ui-editor = Editor
settings-ui-journals = Journals
settings-ui-files = Files
settings-ui-backups-recovery = Backups & recovery
settings-ui-graph = Graph
settings-ui-plugins = Plugins
settings-ui-help-diagnostics = Help & diagnostics
settings-ui-keyboard-shortcuts = Keyboard shortcuts
settings-ui-about = About
settings-ui-settings = Settings
settings-ui-built = Built { $value1 }
settings-ui-search-shortcuts = Search shortcuts…
settings-ui-search-settings = Search settings…
settings-ui-search-shortcuts-2 = Search shortcuts
settings-ui-search-settings-2 = Search settings
settings-ui-restore-settings-size = Restore settings size
settings-ui-maximize-settings = Maximize settings
settings-ui-config-edn-could-not-be-read-this-graph-is = config.edn could not be read. This graph is read-only. Repair the config and reopen the graph.
settings-ui-no-matching-settings = No matching settings
settings-ui-tine-s-default-differs-from-logseq-here = Tine's default differs from Logseq here
settings-ui-differs-from-logseq = Differs from Logseq
settings-og-reference-spacing = Logseq: no space — the caret sits right after the closing brackets.
settings-og-copy-subtree = Logseq: always copies a selected block's whole sub-tree.
# Keep collapsed:: and id:: unchanged: they are property keys.
settings-og-copy-collapsed = Logseq: keeps collapsed:: in the copied text (only id:: is stripped).
settings-og-reference-zoom = Logseq: zooms into the referenced block on click.
settings-ui-match-logseq = ↩ Match Logseq
settings-ui-could-not-load-advanced-section-preference = Could not load Advanced section preference.
settings-ui-could-not-save-advanced-section-preference = Could not save Advanced section preference.
settings-ui-advanced = <slot1>{ $choice1 ->
        [yes] ▾
       *[no] ▸
    }</slot1> Advanced
settings-ui-saved-restart-tine-to-apply-the-window-frame-change = Saved. Restart Tine to apply the window-frame change.
settings-ui-couldn-t-save-the-window-frame-setting = Couldn't save the window-frame setting. ({ $value1 })
settings-ui-mode = Mode
settings-ui-accent-color = Accent color
settings-ui-reset = Reset
settings-ui-interface-size = Interface size
settings-ui-zoom-the-whole-interface-ctrl-ctrl-ctrl-0-or = Zoom the whole interface — Ctrl + / Ctrl − / Ctrl 0, or Ctrl + scroll. Saved on this device. (Over/within the PDF pane, those zoom the PDF instead.)
settings-ui-smaller-ctrl = Smaller (Ctrl −)
settings-ui-larger-ctrl = Larger (Ctrl +)
settings-ui-wide-mode = Wide mode
settings-ui-drops-the-reading-width-cap = Drops the reading-width cap.
settings-ui-document-mode = Document mode
settings-ui-hides-bullets-and-indent-guides-for-a-cleaner-prose = Hides bullets and indent guides for a cleaner prose view.
settings-ui-document-mode-enter-creates-a-new-block = Document-mode Enter creates a new block
settings-ui-keep-the-normal-enter-new-block-and-shift-enter = Keep the normal Enter = new block and Shift + Enter = line break mapping while Document mode is on. Off (the default) swaps them, like Logseq. Saved to <code>:shortcut/doc-mode-enter-for-new-block?</code> in <code>config.edn</code>.
settings-ui-show-brackets = Show brackets
settings-ui-show-the-around-page-references-saved-to-ui-show = Show the <code>[[ ]]</code> around page references. Saved to <code>:ui/show-brackets?</code> in <code>config.edn</code>; toggle with <code>mod+c mod+b</code>.
settings-ui-typographic-replacements = Typographic replacements
settings-ui-show-arrows-and-dashes-as-glyphs-en-dash-em = Show arrows and dashes as glyphs — `->` → →, `-->` → ⟶, `--` → – (en dash), `---` → — (em dash). “While reading” keeps your Markdown as ASCII and only changes the rendered view (like `\Delta` → Δ); “While typing” rewrites the source itself as you type. Off by default, as in Logseq: text renders as you typed it.
settings-ui-on-while-reading = On (while reading)
settings-ui-on-while-typing = On (while typing)
settings-ui-off = Off
settings-ui-wrap-code-lines = Wrap code lines
settings-ui-wrap-long-code-lines-in-every-code-block-while = Wrap long code lines in every code block while reading and editing. Off by default, as in Logseq. Saved on this device.
settings-ui-auto-pair-brackets-quotes = Auto-pair brackets & quotes
settings-ui-typing-quot-inserts-the-matching-closer-with-the-caret = Typing ( [ { "{" } " ` inserts the matching closer with the caret between, wraps a selection, types through a closer, and Backspace on an empty pair clears both. (Page-ref `[[ ]]` always auto-closes.) On by default, as in Logseq.
settings-ui-space-after-inserting-a-reference = Space after inserting a reference
settings-ui-after-you-pick-a-page-from-or-a-block = After you pick a page from [[…]] or a block from ((…)) autocomplete, the caret lands past the closing brackets. ON (Tine default) also drops a space there so the next word flows on without stepping over the brackets; a block-final space is trimmed on save, so it never persists.
settings-ui-dim-in-focus-mode = Dim in focus mode
settings-ui-auto-enable-dim-inactive-blocks-t-b-when-entering = Auto-enable dim inactive blocks (t b) when entering focus mode (t f).
settings-ui-load-local-file-images = Load local-file images
settings-ui-let-raw-html-tags-in-notes-load-images-from = Let raw-HTML <img> tags in notes load images from absolute paths anywhere on this computer (e.g. an imported note's <img src="/home/…/pic.png">). Off by default — this is a permission: a synced or imported note isn't self-authored, so only enable it for graphs you trust. In-graph images and https images always work regardless.
settings-ui-smooth-scrolling-experimental = Smooth scrolling (experimental)
settings-ui-animate-the-journal-feed-s-scrolling-to-smooth-out = Animate the journal feed's scrolling to smooth out WebKitGTK's stepped mouse-wheel jumps. Off by default; this is a feel experiment — turn it off if it gets in the way.
settings-ui-system-title-bar-window-controls = System title bar & window controls
settings-ui-use-your-os-s-native-window-frame-title-bar = Use your OS's native window frame (title bar, minimize/maximize/close, rounded corners) instead of Tine's compact built-in controls. Restart Tine after changing this setting. Off by default — the built-in controls save a row of vertical space.
settings-ui-window-controls = Window controls
settings-ui-macos-draws-tine-with-native-rounded-corners-and-traffic = macOS draws Tine with native rounded corners and traffic-light buttons.
settings-ui-native-macos = Native (macOS)
settings-ui-new-journal-template = New-journal template
settings-ui-template-inserted-into-a-new-day-s-journal-saved = Template inserted into a new day's journal. Saved to <code>:default-templates { "{" }:journals …{ "}" }</code> in <code>config.edn</code>. “(none)” → blank days (the default). Make a template via a block's right-click menu.
settings-ui-none-blank-days = (none) — blank days
settings-ui-edit = Edit →
settings-ui-file-format = File format
settings-ui-format-for-new-pages-and-journals-markdown-or-org = Format for <strong>new</strong> pages and journals — Markdown or Org. Existing <code>.md</code> and <code>.org</code> files keep their own format and are edited in place. Saved to <code>:preferred-format</code> in <code>config.edn</code>.
settings-ui-markdown = Markdown
settings-ui-org = Org
settings-ui-spell-checker = Spell checker
settings-ui-underline-misspelled-words-while-editing-with-right-click-suggestions = Underline misspelled words while editing, with right-click suggestions and “add to dictionary” (uses the system spell checker). <strong>On by default</strong>, like Logseq. Applies live — no restart needed.
settings-ui-logical-outdenting = Logical outdenting
settings-ui-move-an-outdented-block-after-its-parent-while-leaving = Move an outdented block after its parent while leaving following siblings in place. Off (the default) reparents those siblings beneath the moved block. Saved to <code>:editor/logical-outdenting?</code> in <code>config.edn</code>.
settings-ui-spellcheck-languages = Spellcheck languages
settings-ui-tick-the-dictionaries-to-check-several-at-once-is = Tick the dictionaries to check — several at once is fine (e.g. English + Czech), and a word valid in <em>any</em> ticked language isn’t flagged, so bilingual notes don’t squiggle. <strong>None ticked → follows your OS locale</strong> (all Logseq can do). Dictionaries are discovered from the system; install more with your package manager (e.g. <code>hunspell-cs</code>), then Rescan.
settings-ui-not-installed = not installed
settings-ui-no-dictionaries-found-install-one-e-g-hunspell-en = No dictionaries found. Install one (e.g. <code>hunspell-en-us</code>), then Rescan.
settings-ui-rescan = ↻ Rescan
settings-ui-link-autocomplete-default = Link autocomplete default
settings-ui-controls-enter-for-non-exact-name-and-name-completion = Controls Enter for non-exact [[name and #name completion. OG adaptive (default) picks the shortest lexical strict-prefix match and puts Create immediately after it; fuzzy-only matches leave Create first. Prefer existing always leads with a match; Prefer exactly what I typed leads with Create. Exact existing names always select the existing page.
settings-ui-og-adaptive = OG adaptive
settings-ui-prefer-existing = Prefer existing
settings-ui-prefer-exactly-what-i-typed = Prefer exactly what I typed
settings-ui-switch-to-an-already-open-tab-when-navigating = Switch to an already-open tab when navigating
settings-ui-plain-navigation-to-a-page-journal-or-exact-zoomed = Plain navigation to a page, journal, or exact zoomed/file-pinned view focuses the matching tab if one is already open. Middle-click and explicit Open in new tab still create another tab.
settings-ui-learn-ctrl-k-choices = Learn Ctrl+K choices
settings-ui-after-you-deliberately-open-the-same-result-more-than = After you deliberately open the same result more than once for a query, Ctrl+K may prefer it only among equally strong matches. History stays on this device and in this graph; saved searches and queries remain deterministic.
settings-ui-ctrl-k-ranking-reset-for-this-graph = Ctrl+K ranking reset for this graph
settings-ui-reset-ranking = Reset ranking
settings-ui-copy-a-parent-block-s-sub-blocks = Copy a parent block's sub-blocks
settings-ui-when-you-copy-cut-a-selected-block-that-has = When you copy/cut a selected block that has children: ON copies the whole sub-tree; OFF copies only the block(s) you actually selected. Tine defaults to OFF because selecting just the parent and getting its entire tree is surprising.
settings-ui-strip-collapsed-when-copying = Strip collapsed:: when copying
settings-ui-a-collapsed-block-carries-a-hidden-collapsed-true-property = A collapsed block carries a hidden collapsed:: true property (view state, not content). Tine defaults to ON (drops it from copied text for a cleaner paste); OFF keeps it. (id:: is always stripped from copies, like Logseq.)
settings-ui-click-a-block-reference-to-zoom-in = Click a block reference to zoom in
settings-ui-plain-clicking-an-inline-block-reference-on-zooms-into = Plain-clicking an inline ((block reference)): ON zooms into the referenced block (opens it as its own page, like Logseq); OFF (Tine default) scrolls to it in place and flashes it. Shift-click always opens it in the sidebar.
settings-ui-journal-date-format = Journal date format
settings-ui-how-journal-dates-are-displayed-and-how-new-date = How journal dates are displayed and how new <code>[[date]]</code> titles are written. Display-only — your journal <em>file names</em> are untouched and existing journals keep working. Saved to <code>:journal/page-title-format</code>.
settings-ui-first-day-of-week = First day of week
settings-ui-starting-column-of-the-calendar-and-the-scheduled-deadline = Starting column of the calendar and the scheduled/deadline date pickers. Saved to <code>:start-of-week</code> in <code>config.edn</code>(Logseq’s setting), so it travels with the graph.
settings-ui-monday = Monday
settings-ui-tuesday = Tuesday
settings-ui-wednesday = Wednesday
settings-ui-thursday = Thursday
settings-ui-friday = Friday
settings-ui-saturday = Saturday
settings-ui-sunday = Sunday
settings-ui-show-carry-over-buttons = Show carry-over buttons
settings-ui-show-the-carry-buttons-next-to-journal-titles-off = Show the carry buttons next to journal titles. Off → use the right-click menu instead.
settings-ui-carry-over-keeps-context = Carry-over keeps context
settings-ui-move-whole-blocks-that-contain-an-open-task-on = Move whole blocks that contain an open task (on) vs. pull out just the task (off).
settings-ui-carry-over-header = Carry-over header
settings-ui-add-a-carried-over-heading-above-carried-tasks = Add a “Carried over” heading above carried tasks.
settings-ui-carry-last-n-days = Carry “last N days”
settings-ui-n-for-the-carry-last-n-days-button-on = N for the “Carry last N days” button on today’s journal (and the Ctrl-K command).
settings-ui-task-workflow = Task workflow
settings-ui-which-markers-tab-cycle-through-and-what-new-tasks = Which markers Tab/⌘↵ cycle through and what new tasks use. Saved to <code>:preferred-workflow</code> in <code>config.edn</code>, so it travels with the graph.
settings-ui-todo-doing = TODO / DOING
settings-ui-now-later = NOW / LATER
settings-ui-time-tracking = Time tracking
settings-ui-marker-transitions-write-og-compatible-logbook-clock-rows-saved = Marker transitions write OG-compatible <code>:LOGBOOK:</code> CLOCK rows. Saved to <code>:feature/enable-timetracking?</code>; seconds mode follows <code>:logbook/settings</code> and is { $choice1 ->
        [yes] on
       *[no] off
    }.
settings-ui-quick-capture-enter-key = Quick-capture Enter key
settings-ui-in-the-quick-capture-window-on-enter-files-the = In the quick-capture window: ON → Enter files the capture. OFF → Enter starts a new block; the “Quick-capture: file to today’s journal” shortcut files (default Ctrl+Shift+Enter, remappable under Keyboard shortcuts). Ctrl+Enter stays free for cycling the task marker.
settings-ui-agenda-window = Agenda window
settings-ui-today-s-scheduled-deadline-list-shows-items-whose-scheduled = Today’s “Scheduled & Deadline” list shows items whose scheduled/deadline date is within this window; older/further ones are hidden.
settings-ui-days-back = days back ·
settings-ui-couldn-t-save = Couldn't save: { $value1 }
settings-ui-tine-snapshots-your-graph-s-markdown-to-a-local = Tine snapshots your graph’s markdown to a local folder each time it opens (outside the graph, so Syncthing never sees it). A safety net against a bad write — independent of OG Logseq’s own backups.
settings-ui-snapshots-to-keep = Snapshots to keep
settings-ui-oldest-snapshots-beyond-this-are-pruned = Oldest snapshots beyond this are pruned.
settings-ui-refresh = Refresh
settings-ui-loading-snapshot-settings = Loading snapshot settings…
settings-ui-retry = Retry
settings-ui-no-snapshots-yet = No snapshots yet.
settings-ui-files-2 = { $files } files
settings-ui-restore = Restore
settings-ui-couldn-t-do-that = Couldn’t do that: { $value1 }
settings-ui-move-the-journal-file-to-the-trash-it-s = Move the journal file “{ $name }” to the trash?

    It's a duplicate of another file for the same day. It moves to logseq/.tine-trash (recoverable).
settings-ui-duplicate-journal-days = Duplicate journal days
settings-ui-these-days-have-more-than-one-file-e-g = These days have more than one file (e.g. a <code>2026_06_26.org</code> and a title-named <code>Friday, 26-06-2026.org</code>) — usually left over from changing the date format. <strong>Open</strong> reaches a file directly (it's editable and saves back to itself); <strong>Merge</strong> folds a stray into the canonical day; <strong>Rename</strong> turns it into a normal page; <strong>Trash</strong> removes the redundant one (recoverable).
settings-ui-conflicts = Conflicts
settings-conflicts-summary =
    { $count ->
        [one] { $count } item needs a decision: sync conflict copies or version-control merge markers. The Conflicts page lists them, with <strong>Discard copy</strong> for sync copies; the <strong>N conflicts</strong> badge in the sidebar opens it too.
       *[other] { $count } items need a decision: sync conflict copies or version-control merge markers. The Conflicts page lists them, with <strong>Discard copy</strong> for sync copies; the <strong>N conflicts</strong> badge in the sidebar opens it too.
    }
settings-ui-open-conflicts = Open conflicts
settings-ui-asset-names = Asset names
settings-ui-new-asset-filename = New asset filename
settings-ui-how-files-are-named-when-you-paste-drag-or = How files are named when you paste, drag, or insert media into <code>assets/</code>. Tokens: <code>%assetname</code> (the original file’s name), <code>%ext</code>, <code>%yyyymmdd</code>, <code>%hhmmss</code> — also granular <code>%yyyy</code> <code>%MM</code> <code>%dd</code> <code>%HH</code> <code>%mm</code> <code>%ss</code>. Anything else is literal. A clipboard paste has no name, so <code>%assetname</code> falls back to a timestamp. Device-local; Logseq keeps the original name for dragged files.
settings-ui-original-name = Original name
settings-ui-date-name = Date + name
settings-ui-drag-holiday-photo-jpg = Drag <slot1>Holiday Photo.JPG</slot1> → <slot2>{ $value1 }</slot2>
settings-ui-paste-an-image = Paste an image → <slot1>{ $value1 }</slot1>
settings-ui-watch-for-external-edits = Watch for external edits
settings-ui-how-tine-notices-changes-made-outside-it-og-logseq = How Tine notices changes made outside it (OG Logseq, Syncthing, an editor). <slot1>Live</slot1> uses the OS file watcher (inotify) — no idle CPU wakeups; the right choice on a normal local disk. <slot2>Poll</slot2> rescans every 3 seconds — only needed on filesystems where inotify is unreliable (some network/NFS mounts). Saved per device.
settings-ui-live-inotify = Live (inotify)
settings-ui-poll-3s = Poll (3s)
settings-ui-found = Found: { $command }
settings-ui-couldn-t-find-it-set-the-command-manually = Couldn’t find it — set the command manually.
settings-ui-autodetect-failed = Autodetect failed.
settings-ui-diagram-editors = Diagram editors
settings-ui-edit-diagram-assets-in-your-own-installed-app-a = Edit diagram assets in your own installed app. A <slot1>/drawio</slot1> command creates a new editable <code>.drawio.svg</code>; hovering any matching image shows an “Edit in …” button. Leave a command blank to use the system default opener. A <slot2>{ "{" }{ "}" }</slot2> in the command is replaced by the file path (otherwise it’s appended). Wrap a program or argument containing spaces in double quotes. Desktop only; device-local.
settings-ui-system-default-opener = system default opener
settings-ui-some-pages-couldn-t-be-saved-resolve-conflicts-before = Some pages couldn't be saved — resolve conflicts before scanning assets.
settings-ui-couldn-t-open = Couldn’t open { $name }: { $value1 }
settings-ui-is-used-by-a-page-now-so-it-was = { $name } is used by a page now, so it was kept. Scan again to refresh the list.
settings-ui-moved-to-trash = Moved { $name } to trash
settings-ui-couldn-t-trash = Couldn’t trash: { $value1 }
settings-asset-trash-confirmation =
    { $count ->
        [one] Permanently delete { $count } asset file ({ $size }) in the trash?

            This cannot be undone. Page, journal, and conflict recovery files in logseq/.tine-trash will be kept.
       *[other] Permanently delete { $count } asset files ({ $size }) in the trash?

            This cannot be undone. Page, journal, and conflict recovery files in logseq/.tine-trash will be kept.
    }
settings-asset-trash-emptied =
    { $count ->
        [one] Emptied asset trash ({ $count } file)
       *[other] Emptied asset trash ({ $count } files)
    }
settings-ui-couldn-t-empty-trash = Couldn’t empty trash: { $value1 }
settings-ui-permanently-delete-asset-files-in-logseq-tine-trash = Permanently delete asset files in logseq/.tine-trash
settings-ui-empty-asset-trash = Empty asset trash ({ $count })
# A list of recovery files retained when asset trash is emptied. Zero categories
# are omitted; before* flags insert a separator only after a nonempty category.
settings-trash-protected-summary = Protected recovery trash kept: { settings-trash-protected-pages }{ settings-trash-protected-journals }{ settings-trash-protected-conflicts }{ settings-trash-protected-other }
settings-trash-protected-pages =
    { $pages ->
        [0] { "" }
        [one] { $pages } page
       *[other] { $pages } pages
    }
settings-trash-protected-journals =
    { $journals ->
        [0] { "" }
       *[other] { $beforeJournals ->
            [1] { ", " }
           *[0] { "" }
        }{ $journals ->
            [one] { $journals } journal
           *[other] { $journals } journals
        }
    }
settings-trash-protected-conflicts =
    { $conflicts ->
        [0] { "" }
       *[other] { $beforeConflicts ->
            [1] { ", " }
           *[0] { "" }
        }{ $conflicts ->
            [one] { $conflicts } conflict
           *[other] { $conflicts } conflicts
        }
    }
settings-trash-protected-other =
    { $other ->
        [0] { "" }
       *[other] { $beforeOther ->
            [1] { ", " }
           *[0] { "" }
        }{ $other } other
    }
settings-ui-files-in-assets-that-no-block-links-to-deleting = Files in <code>assets/</code> that no block links to. Deleting a block never deletes its media (a safety net), so unused files can accumulate — review and trash them here. Trashed files move to <code>logseq/.tine-trash</code> (recoverable); click a name to open it in your default app.
settings-ui-no-orphaned-media-every-asset-is-referenced = No orphaned media — every asset is referenced. 🎉
settings-orphaned-assets-summary =
    { $count ->
        [one] { $count } orphan · { $size } reclaimable
       *[other] { $count } orphans · { $size } reclaimable
    }
settings-ui-open-in-the-default-app = Open in the default app
settings-ui-trash = Trash

# Helper-return and component-prop interface text (GH #107).
settings-prose-always-ask-before-applying-an-external-change = Always ask before applying an external change
settings-prose-when-a-page-you-are-not-editing-changes-on-disk = When a page you are not editing changes on disk, keep what you are reading and offer Reload from disk / Keep mine instead of updating it. Changes that conflict or arrive mid-edit are always handled safely either way.
settings-prose-re-reads-every-file-in-the-open-graph-and-rebuilds = Re-reads every file in the open graph and rebuilds Tine's view of it.
settings-prose-show-tine-in-the-system-tray = Show Tine in the system tray
settings-prose-adds-a-tray-notification-area-macos-menu-bar-icon-click = Adds a tray / notification-area (macOS: menu-bar) icon. Click it to show or hide the main window; its menu has Open Tine, Quick Capture and Quit. Closing the window still quits Tine. Off by default.
settings-prose-minimize-to-tray = Minimize to tray
settings-prose-minimizing-the-main-window-hides-the-window-instead-of-leaving = Minimizing the main window hides the window instead of leaving it in the taskbar; restore it from the tray icon. Needs “Show Tine in the system tray”.
settings-prose-start-minimized-to-tray = Start minimized to tray
settings-prose-launch-with-the-main-window-hidden-open-it-from-the = Launch with the main window hidden; open it from the tray icon. Launching Tine again, or opening a graph or link, always shows it. Needs “Show Tine in the system tray”.

settings-prose-last-rescan = Last rescan finished at { $time }.
settings-prose-file-size = { $value } { $unit }
