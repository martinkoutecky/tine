
# Interface text; variables carry untranslated names, paths or diagnostic details.
app-choose-no-to-review-retry-saving = { app-close-explanation }{ "\u000A" }{ "\u000A" }{ app-close-inventory }{ "\u000A" }{ "\u000A" }Choose No to review, retry saving, or copy your drafts. Close this window anyway and lose them?
app-unsaved-changes = Unsaved changes
app-couldn-t-save-pending-pdf-changes-the-graph = Couldn't save pending PDF changes. The graph remains open.
app-still-saving-your-changes-closing-in = Still saving your changes — closing in a moment.
app-couldn-t-confirm-closing-the-window-your-unsaved = Couldn't confirm closing the window. Your unsaved changes are still open.
app-tine-couldn-t-close-the-android-activity-tap = Tine couldn't close the Android activity. Tap Back to retry closing.
app-close-tab = Close tab
app-pane-select = Pane select
app-move-onto-seams-edges = <kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> move (onto seams & edges) · <kbd>Enter</kbd> enter pane · <kbd>Del</kbd> close pane
app-ctrl-k-open-a-page-in-this-pane = <kbd>Ctrl+K</kbd> open a page in this pane · <kbd>Esc</kbd> exit

app-captured-to = Captured to “{ $title }”
app-captured-to-today-s-journal = Captured to today's journal
app-capture-couldn-t-be-saved-its-text = Capture couldn't be saved — its text is kept in the capture window
app-tine-was-renamed-under-the-hood-so = Tine was renamed under the hood, so we moved your settings and backups across. A few app-level preferences (e.g. keyboard shortcuts) might need setting again — sorry about that!
# $fallbackDirectory is the temporary application-data directory, not a graph path.
app-tine-could-not-write-its-usual-application-data = Tine could not write its usual application-data folder, so this session is keeping settings and backups in { $fallbackDirectory } instead. Fixing the permissions on that folder restores the normal location.
app-quick-capture-could-not-connect-to-this = Quick Capture could not connect to this graph window.
app-the-block-renderer-failed-to-load = The block renderer failed to load — text is shown unformatted. Please reload Tine; if this persists, report it.
app-finishing-graph-operation = Finishing graph operation…
app-close = Close
app-close-navigation-sidebar = Close navigation sidebar
app-journals = Journals
app-toggle-theme-t-t = Toggle theme (t t)
app-toggle-right-sidebar-t-r = Toggle right sidebar (t r)
app-settings-t-s = Settings (t s)
app-refreshing-changes-from-disk = Refreshing changes from disk…
app-exit-focus-esc = Exit focus (Esc)

app-close-explanation = { $reason ->
    [still-saving] Tine is still writing your changes and is taking longer than expected — a slow or network drive can do this.
   *[other] Tine has changes that could not be saved (a conflict or a stuck save).
    }
app-close-inventory = { $inventory ->
    [empty] Pending attachments or storage work; no page draft identified.
   *[other] { $inventory }
    }
# Key names are literal shortcut notation. Inline slots retain the pane hint's emphasis.
# $kind selects an outer window edge, a pane edge, or a seam between panes. <em-hint> highlights the split target and page-entry hint; <span> groups the instruction.
app-pane-split-instruction = { $kind ->
    [edge] <kbd>Enter</kbd> <span>split the <em-hint>whole window</em-hint></span> · <em-hint>type a page name</em-hint> (or <kbd>Ctrl+K</kbd>) to open it in the new split
    [pane-edge] <kbd>Enter</kbd> <span>split <em-hint>this pane</em-hint></span> · <em-hint>type a page name</em-hint> (or <kbd>Ctrl+K</kbd>) to open it in the new split
   *[seam] <kbd>Enter</kbd> <span>split here (mirrors the pane)</span> · <em-hint>type a page name</em-hint> (or <kbd>Ctrl+K</kbd>) to open it in the new split
    }
# $kind selects the pane-edge navigation hint; <span> groups that optional hint.
app-pane-split-navigation = { $kind ->
    [pane-edge] <span>press outward again to widen the split · </span><kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> move · <kbd>Esc</kbd> exit
   *[other] <kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> move · <kbd>Esc</kbd> exit
    }

# Helper-return and component-prop interface text (GH #107).
app-prose-navigation-sidebar = Navigation sidebar
app-prose-unexpected-failure = Unexpected failure.
