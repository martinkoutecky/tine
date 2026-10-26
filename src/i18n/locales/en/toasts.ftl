## Toasts: the notification stack's own controls and notes
image-copy-progress = Copying image…
toast-copy = Copy
toast-copy-title = Copy the full message
toast-dismiss = Dismiss
# $count (number): how many times this same error was shown.
toast-repeat-count = shown { $count } times
toast-copied = Error message copied
toast-copy-failed = Couldn't copy the message; select its text instead.
toast-graph-folder-reveal-failed = Could not show the graph folder: { $error }
toast-link-outside-graph = { $dest } points outside the graph folder, so Tine does not open it.
toast-wsl-rootfs-warning = This graph is inside a WSL1 rootfs. Windows access can corrupt Linux files. Open it through \\wsl.localhost\<distro>\… instead.
# $progress (string): `tine serve`'s progress as it reports it.
toast-graph-open-owner-waiting = Finishing the command line's work on this graph ({ $progress })…

# Application notices. Error/detail variables carry untranslated diagnostics; names and paths are graph content.
asset-landing-the-asset-was-saved-but-it-was-not = The asset was saved, but it was not inserted because the graph or block changed.

asset-landing-saved-to-the-assets-of-graph-it-was = Saved { $stored } to the assets/ of graph “{ $graphName }”; it was not inserted because the graph changed.

backup-restore-some-pages-couldn-t-be-saved-resolve-conflicts = Some pages couldn't be saved — resolve conflicts before restoring.

backup-restore-snapshot-restored-but-the-graph-couldn-t-be = Snapshot restored, but the graph couldn't be reloaded. Reopen it to see the restored files.

backup-restore-restore-failed-part-way = Restore failed part-way: { $error }

backup-restore-restored-snapshot-from = Restored snapshot from { $when }

# $error is an untranslated diagnostic from the operating system or backend.
backup-restore-restore-failed = Restore failed: { $error }

binding-some-state-of-the-previous-graph-could-not = Some state of the previous graph could not be cleared (binding.clear, { $failures } failed).

binding-was-not-saved-it-was-opened-in-a = { $what } was not saved: it was opened in a graph that is no longer open.

block-ref-actions-select-blocks-or-edit-one-to-copy-their = Select blocks (or edit one) to copy their block refs

block-ref-actions-too-much-selected-to-duplicate = Too much selected to duplicate

carry-carry-couldn-t-be-saved-resolve-the-conflict = Carry couldn't be saved — resolve the conflict; your moved tasks are kept in the editor.

# $error is an untranslated diagnostic from the operating system or backend.
carry-could-not-read-journal-days-for-carry = Could not read journal days for carry: { $error }

carry-no-previous-day-with-content-to-carry-from = No previous day with content to carry from

# $error is an untranslated diagnostic from the operating system or backend.
carry-could-not-carry-tasks = Could not carry tasks: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
code-display-could-not-load-code-wrapping = Could not load code wrapping: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
code-display-could-not-remember-code-wrapping = Could not remember code wrapping: { $error }

conflict-overview-discarded = Discarded { $name }

# $error is an untranslated diagnostic from the operating system or backend.
conflict-overview-couldn-t-discard-it = Couldn’t discard it: { $error }

conflict-resolution-finish-the-current-edit-then-apply-this-resolution = Finish the current edit, then apply this resolution.

conflict-resolution-resolved-the-conflict-in = Resolved the conflict in “{ $page_name }”

conflict-resolution-applied-the-resolution-to-but-it-is-not = Applied the resolution to “{ $page_name }”, but it is not saved yet. Tine keeps trying to save it.

conflict-resolution-resolve-this-page-s-save-conflict-first-then = Resolve this page’s save conflict first, then apply this resolution.

conflict-resolution-the-file-changed-on-disk-re-reading-it = The file changed on disk — re-reading it, please redo your choices.

# $error is an untranslated diagnostic from the operating system or backend.
conflict-resolution-couldn-t-resolve-it = Couldn’t resolve it: { $error }

conflict-resolution-still-has-unresolved-conflicts = “{ $page }” still has unresolved conflicts

# $error is an untranslated diagnostic from the operating system or backend.
conflict-resolution-couldn-t-do-that = Couldn’t do that: { $error }

diagnostics-tab-could-not-create-the-diagnostic-report = Could not create the diagnostic report.

diagnostics-tab-no-graph-rescan-ran-open-a-graph-first = No graph rescan ran. Open a graph first.

diagnostics-tab-could-not-rescan-the-graph = Could not rescan the graph.

diagnostics-tab-diagnostic-report-copied = Diagnostic report copied

diagnostics-tab-could-not-copy-the-diagnostic-report = Could not copy the diagnostic report.

diagnostics-tab-diagnostic-report-saved = Diagnostic report saved

diagnostics-tab-could-not-save-the-diagnostic-report = Could not save the diagnostic report.

diagnostics-tab-recorded-diagnostic-events-cleared = Recorded diagnostic events cleared

diagnostics-tab-could-not-clear-the-recorded-diagnostic-events = Could not clear the recorded diagnostic events.

error-toast-history-error-message-copied = Error message copied

error-toast-history-could-not-copy-the-error-message = Could not copy the error message.

export-modal-copied-to-clipboard = Copied to clipboard

export-modal-couldn-t-copy-clipboard-write-failed = Couldn't copy: clipboard write failed.

# $error is an untranslated diagnostic from the operating system or backend.
external-link-couldn-t-open = Couldn't open { $dest }. ({ $error })

failure-boundary-could-not-be-displayed = { $region } could not be displayed.

failure-boundary-couldn-t-copy-error-details-clipboard-write-failed = Couldn’t copy error details: clipboard write failed.

graph-verification-graph-verification-was-incomplete = Graph verification was incomplete

graph-verification-graph-verification-report-copied = Graph verification report copied

graph-verification-could-not-copy-the-graph-verification-report = Could not copy the graph verification report.

graph-verification-graph-verification-report-saved = Graph verification report saved

graph-verification-could-not-save-the-graph-verification-report = Could not save the graph verification report.

graph-verification-could-not-compare-the-reports = Could not compare the reports: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
home-page-setting-could-not-save-home-page = Could not save home page: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
journal-filename-panel-couldn-t-list-journal-files-to-rename = Couldn’t list journal files to rename: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
journal-filename-panel-couldn-t-rename-them = Couldn’t rename them: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
property-type-could-not-save-property-type = Could not save property type: { $error }

quick-switcher-the-graph-changed-before-the-page-was-created = The graph changed before the page was created. Try again.

# $error is an untranslated diagnostic from the operating system or backend.
quick-switcher-could-not-create-it-will-be-saved-on = Could not create “{ $name }”: { $error }. It will be saved on your first edit.

# $error is an untranslated diagnostic from the operating system or backend.
quick-switcher-could-not-create = Could not create “{ $name }”: { $error }

quick-switcher-that-pane-is-no-longer-available-close-this = That pane is no longer available. Close this search and try again.

# $error is an untranslated diagnostic from the operating system or backend.
sidebar-couldn-t-remove-graph = Couldn't remove graph. ({ $error })

# $error is an untranslated diagnostic from the operating system or backend.
sidebar-couldn-t-open-the-graph-picker = Couldn't open the graph picker. ({ $error })

# $error is an untranslated diagnostic from the operating system or backend.
sidebar-couldn-t-create-the-graph = Couldn't create the graph. ({ $error })

toasts-image-copied = Image copied

toasts-couldn-t-copy-the-image = Couldn't copy the image

# $error is an untranslated diagnostic from the operating system or backend.
workspace-switcher-couldn-t-delete-workspace = Couldn't delete workspace: { $error }

block-link-copy-couldn-t-copy-block-save-or-clipboard-write = Couldn't copy block { $kind }: save or clipboard write failed.

block-link-copy-copied-tine-link = Copied Tine link

# $error is an untranslated diagnostic from the operating system or backend.
block-link-copy-couldn-t-copy-tine-link = Couldn't copy Tine link: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
graph-row-menu-could-not-show-the-graph-folder = Could not show the graph folder: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
graph-row-menu-could-not-copy-graph-path = Could not copy graph path: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
graph-row-menu-could-not-remove-graph = Could not remove graph: { $error }

page-trailing-could-not-add-a-block-to-this-page = Could not add a block to this page.

# $error is an untranslated diagnostic from the operating system or backend.
content-width-could-not-remember-the-page-width = Could not remember the page width: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
content-width-could-not-load-the-page-width = Could not load the page width: { $error }

debug-debug-log-unavailable = Debug log unavailable.

debug-tine-did-not-close-cleanly-last-time-a = Tine did not close cleanly last time. A privacy-safe diagnostic report is available.

debug-debug-logging-is-on = Debug logging is ON → { $path }

# $error is an untranslated diagnostic from the operating system or backend.
deep-link-navigation-couldn-t-open-tine-link = Couldn't open Tine link: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
deep-link-navigation-couldn-t-receive-tine-link = Couldn't receive Tine link: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
deep-link-navigation-couldn-t-listen-for-tine-links = Couldn't listen for Tine links: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
defender-hint-could-not-run-the-exclusion = Could not run the exclusion: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
defender-hint-could-not-remember-that-you-closed-the-hint = Could not remember that you closed the hint: { $error }

blocks-outline-is-too-deep-to-insert-a-child = Outline is too deep to insert a child

blocks-outline-is-too-deep-to-move = Outline is too deep to move

blocks-outline-is-too-deep-to-merge = Outline is too deep to merge

moves-outline-is-too-deep-to-move = Outline is too deep to move

paste-pasted-outline-is-too-deep = Pasted outline is too deep

paste-pasted-as-a-copy-references-to-these-blocks = Pasted as a copy; references to these blocks will not follow.

properties-page-header-properties-must-contain-only-valid-key = Page-header properties must contain only valid key:: value lines before they can be saved.

# $line is the 1-based line number inside the page-properties block; $text is that line, shortened.
properties-page-header-bad-line = Page properties can't be saved yet: line { $line } of the properties block, "{ $text }", is not a key:: value property. Fix or remove it.

properties-page-header-has-child-blocks = Page properties can't be saved yet: the properties block has child blocks. Outdent or delete them.

selection-outline-is-too-deep-to-indent = Outline is too deep to indent

# $error is an untranslated diagnostic from the operating system or backend.
graph-rewrite-rename-failed = Rename failed: { $error }

history-undo-history-for-this-page-was-discarded-the = Undo history for this page was discarded: the page was reloaded since those edits

history-redo-skipped-a-block-with-the-same-id = Redo skipped: a block with the same id now exists

# $error is an untranslated diagnostic from the operating system or backend.
favorites-could-not-read-the-favorites-page = Could not read the Favorites page "{ $page }": { $error }

favorites-recovered-the-favorites-page-from-an-interrupted-save = Recovered the Favorites page "{ $name }" from an interrupted save; repeat your last change.

filedrop-drop-a-file-onto-a-block-to-insert = Drop a file onto a block to insert it.

filedrop-has-cells-csv-tsv-drops-are-limited-to = "{ $fileName }" has { $cells } cells; CSV/TSV drops are limited to { $cellLimit }.

filedrop-dropped-files-could-not-be-inserted-at-this = Dropped files could not be inserted at this outline depth.

# $error is an untranslated diagnostic from the operating system or backend.
filedrop-couldn-t-insert-dropped-file = Couldn't insert dropped file: { $error }

follow-link-couldn-t-find-the-referenced-block = Couldn't find the referenced block

gpu-software-rendering-is-on-tine-gpu-0-webkit = Software rendering is on (TINE_GPU=0 / WEBKIT_DISABLE_DMABUF_RENDERER) — scrolling may feel slow. Unset it to use your GPU.

gpu-this-app-image-is-rendering-on-the-cpu = This AppImage is rendering on the CPU (its bundled graphics libraries don't match your system), so scrolling may feel slow. The .deb / .rpm packages use your system's GPU drivers and are noticeably faster.

gpu-tine-is-rendering-on-the-cpu-your-gpu = Tine is rendering on the CPU — your GPU's accelerated path is unavailable, so scrolling may feel slow. Check that your graphics drivers / hardware acceleration are working.

graph-open-failure-couldn-t-open-the-graph = Couldn't open the graph. ({ $detail })

graph-preferences-could-not-save = Could not save { $label }.

# $error is an untranslated diagnostic from the operating system or backend.
graph-session-couldn-t-save-the-journal-template-setting = Couldn't save the journal template setting. ({ $error })

# $error is an untranslated diagnostic from the operating system or backend.
guide-couldn-t-open-the-guide = Couldn't open the Guide. ({ $error })

# $error is an untranslated diagnostic from the operating system or backend.
guide-couldn-t-copy-the-guide-into-your-graph = Couldn't copy the Guide into your graph. ({ $error })

guide-could-not-save-guide-announcement-preference = Could not save Guide announcement preference.

guide-new-in-app-guide-learn-sheets-formulas-queries = New: in-app Guide — learn Sheets, formulas & queries.

# $error is an untranslated diagnostic from the operating system or backend.
home-page-couldn-t-open-the-home-page = Couldn't open the home page "{ $name }". ({ $error })

inpage-replace-nothing-was-replaced-this-page-cannot-be-edited = Nothing was replaced: this page cannot be edited right now.

inpage-replace-replace-changes-only-the-page-you-opened-find = Replace changes only the page you opened Find in.

inpage-replace-this-match-is-inside-a-link-or-markup = This match is inside a link or markup; edit the block to change it.

inpage-replace-no-matches-on-this-page-to-replace = No matches on this page to replace.

journal-slash-thatday-is-only-available-on-journal-pages = /thatday is only available on journal pages.

journal-slash-the-date-was-not-inserted-because-the-graph = The date was not inserted because the graph or block changed.

launcher-ranking-could-not-load-adaptive-launcher-ranking-preference = Could not load adaptive launcher ranking preference.

move-notice-moved-in = Moved in { $page }

preference-writes-could-not-save = Could not save { $label }.

preference-writes-could-not-load = Could not load { $label }.

print-pdf-export-stopped-because-some-page-edits-could = PDF export stopped because some page edits could not be saved. Resolve the save conflict and try again.

print-couldn-t-prepare-for-pdf = Couldn't prepare “{ $name }” for PDF

print-print-failed = Print failed

# The detail comes from the print renderer; keep diagnostic/query text as supplied.
print-query-limit = PDF export stopped at the Print query limit. { $detail }

query-export-budget-could-not-load-query-export-size-limit = Could not load query export size limit.

query-page-props-is-no-longer-there-so-its-properties-cannot = "{ $name }" is no longer there, so its properties cannot be edited.

# $error is an untranslated diagnostic from the operating system or backend.
reload-on-focus-tine-couldn-t-finish-checking-for-external-changes = Tine couldn't finish checking for external changes. Editing is available, but reopen the page before relying on it being current. ({ $error })

reload-on-focus-live-file-notifications-are-unavailable-for-this-graph = Live file notifications are unavailable for this graph ({ $message }). Tine checks for external changes every 3 seconds instead.

reload-on-focus-live-file-notifications-are-back-for-this-graph = Live file notifications are back for this graph.

session-live-changes-were-kept-workspace-recovery-was-skipped = Live changes were kept; workspace recovery was skipped.

# $error is an untranslated diagnostic from the operating system or backend.
session-could-not-restore-workspaces = Could not restore workspaces: { $error }

# $error is an untranslated diagnostic from the operating system or backend.
share-ingest-a-shared-item-couldn-t-be-added-to = A shared item couldn't be added to today's journal: { $error }. It is kept and will be retried.

share-ingest-shared-item-retry = A shared item couldn't be added to today's journal. It is kept and will be retried.

# $error is an untranslated diagnostic from the operating system or backend.
share-ingest-couldn-t-read-tine-s-share-inbox = Couldn't read Tine's share inbox: { $error }

sidebar-sizing-could-not-load-sidebar-width = Could not load sidebar width.

sidebar-sizing-could-not-save-sidebar-width = Could not save sidebar width.

sidebar-sizing-could-not-load-right-sidebar-width = Could not load right sidebar width.

sidebar-sizing-could-not-save-right-sidebar-width = Could not save right sidebar width.

smooth-scroll-could-not-load-smooth-scrolling-preference = Could not load smooth scrolling preference.

source-page-is-no-longer-there = "{ $page }" is no longer there. { $not_done }

theme-gallery-could-not-load-gallery-theme = Could not load gallery theme.

theme-preference-could-not-load-theme-preference = Could not load theme preference.

theme-preference-could-not-update-system-bar-appearance = Could not update system bar appearance.

manager-installed-theme-settings-are-invalid = Installed theme settings are invalid.

manager-some-installed-themes-could-not-be-loaded = Some installed themes could not be loaded.

manager-installed-theme-settings-could-not-be-parsed = Installed theme settings could not be parsed.

manager-could-not-load-installed-themes = Could not load installed themes.

tray-the-tray-setting-was-saved-but-could-not = The tray setting was saved but could not be applied now.

workspace-windows-at-most-extra-windows-can-be-open-at = At most { $windowLimit } extra windows can be open at once.

workspace-windows-couldn-t-open-a-new-window = Couldn't open a new window.

workspaces-workspace-changes-are-visible-but-directory-sync-failed = Workspace changes are visible, but directory sync failed; they may not survive a power loss.

zoom-could-not-apply-interface-zoom = Could not apply interface zoom.

zoom-could-not-save-interface-zoom = Could not save interface zoom.

ui-failure-custom-css = Couldn't read custom.css. No custom CSS was applied; if you just edited it, the previous version is still in effect. Fix the file and save it again.

ui-failure-css-draft = Couldn't keep a recovery copy of the CSS you are editing. The file was not saved; copy your changes before closing the editor.

ui-failure-asset-read = Couldn't read a file from this graph's assets. It may be unreadable.

ui-failure-page-refresh = Couldn't refresh a page after a change. Reopen it to see the latest.

ui-failure-logbook = Couldn't read or update a time-tracking entry.

ui-failure-marker-read = Couldn't read a task marker from a block. It is shown as plain text.

ui-failure-page-header-read = Couldn't read a page's property header. It is shown as ordinary text.

ui-failure-export-preview = Couldn't prepare a block for the export preview. It is shown unresolved.

ui-failure-sheet-export = Couldn't read the sheets for export. They are exported as plain outlines.

ui-failure-config-read = Couldn't read config.edn. The graph is open read-only; repair the config and reopen the graph.

ui-failure-conflict-inventory = Couldn't refresh conflicts. The last successful inventory is kept.

ui-failure-graph-verification = Couldn't verify graph files.

ui-failure-unreadable-files =
    { $more ->
        [0] Some page files couldn't be read: { $names }. The rest of the graph is available.
       *[other] Some page files couldn't be read: { $names } and { NUMBER($more, useGrouping: 0) } more (Copy lists them all). The rest of the graph is available.
    }

ui-failure-asset-inventory = Couldn't inspect orphan assets. The last successful scan is kept.

ui-failure-trash-inventory = Couldn't inspect recoverable trash. The last successful count is kept.

ui-failure-backup-read = Couldn't complete the launch backup. Your graph is still open.

ui-failure-backup-feedback = Couldn't receive backup status updates.

ui-failure-page-inventory = Couldn't refresh the page list. The last loaded list is kept.

ui-failure-session-read = Couldn't load the saved session. The current workspace is kept.

ui-failure-template-read = Couldn't load the template list. Try again before creating a template.

ui-failure-template-write = Couldn't create the template. This block is no longer writable.

ui-failure-journal-feed = Couldn't load more journals. Try again.

ui-failure-block-counts = Couldn't refresh block reference counts. The last loaded counts are kept.

ui-failure-block-resolution = Couldn't resolve block references. Try again.

ui-failure-external-link = Couldn't open the link.

ui-failure-capture-preference = Couldn't load the capture setting.

ui-failure-audio-load = Couldn't load this audio file.

ui-failure-audio-play = Couldn't play this audio file.

ui-failure-clipboard-association = Couldn't paste these blocks.

ui-failure-window-state = Couldn't read the window state.

ui-failure-window-action = Couldn't change the window.

ui-failure-pdf-find = Couldn't search this PDF. The search results were cleared.

ui-failure-query-hydration = Couldn't load this query page for editing.

defender-hint-offer = Opening this graph took a while, and Windows Defender is scanning each file as Tine reads it. Excluding this graph's folder from real-time scanning usually makes later opens much faster. This changes a Windows security setting, so it asks for administrator approval and only if you choose it.
defender-hint-action = Add an exclusion for this graph folder

carry-result =
    { $count ->
        [0] No unfinished tasks to carry
        [one] Carried { NUMBER($count, useGrouping: 0) } item to today
       *[other] Carried { NUMBER($count, useGrouping: 0) } items to today
    }
guide-copy-result =
    { $created ->
        [1] Copied the guide into your graph under tine-guide/.
       *[other] The guide is already in your graph - opened it.
    }
conflict-resolution-applied =
    { $source ->
        [vcs-markers] Resolved the merge in “{ $page }”
        [duplicate-journal] Folded the other file into “{ $page }”
       *[other] Merged into “{ $page }”
    }
conflict-resolution-draft-changed = Your draft or the file changed. Review the updated comparison, then apply it again.
conflict-resolution-latest-saved = Your latest edit was saved. Review the updated comparison, then apply it again.
conflict-resolution-file-trashed = Moved { $name } to trash
conflict-resolution-file-renamed = Renamed { $old } → { $name }
share-ingest-captured = Captured to today's journal
share-ingest-rejected =
    { $count ->
        [one] A shared item couldn't be read and was kept aside in Tine's share inbox.
       *[other] { NUMBER($count, useGrouping: 0) } shared items couldn't be read and were kept aside in Tine's share inbox.
    }
session-save-failed = Could not save session: { $detail }
inpage-replace-skipped =
    { $count ->
        [one] Nothing was replaced: { NUMBER($count, useGrouping: 0) } match sit inside links or markup.
       *[other] Nothing was replaced: { NUMBER($count, useGrouping: 0) } matches sit inside links or markup.
    }
inpage-replace-result =
    { $count ->
        [one] { $skipped ->
            [0] Replaced { NUMBER($count, useGrouping: 0) } match.
           *[other] Replaced { NUMBER($count, useGrouping: 0) } match; { NUMBER($skipped, useGrouping: 0) } inside links, markup or read-only blocks left as they were.
        }
       *[other] { $skipped ->
            [0] Replaced { NUMBER($count, useGrouping: 0) } matches.
           *[other] Replaced { NUMBER($count, useGrouping: 0) } matches; { NUMBER($skipped, useGrouping: 0) } inside links, markup or read-only blocks left as they were.
        }
    }
query-export-dialog-exported =
    { $previous ->
        [1] Exported { $pages } pages to { $path } (previous export kept at { $retired })
       *[other] Exported { $pages } pages to { $path }
    }
defender-hint-result =
    { $outcome ->
        [added] Windows Defender now skips this graph's folder. Reopen the graph to feel the difference.
        [declined] No change was made: the administrator prompt was not approved. The offer returns the next time this graph opens slowly.
       *[other] Windows Defender did not accept the exclusion. { $detail }
    }

filedrop-inserted =
    { $count ->
        [one] Inserted { NUMBER($count, useGrouping: 0) } file
       *[other] Inserted { NUMBER($count, useGrouping: 0) } files
    }
journal-filename-panel-renamed =
    { $count ->
        [one] Renamed { NUMBER($count, useGrouping: 0) } journal file
       *[other] Renamed { NUMBER($count, useGrouping: 0) } journal files
    }
toasts-reference-change =
    { $known ->
        [0] { $transferred ->
            [1] References may now point to this block
           *[other] References may now be broken
        }
       *[other] { $transferred ->
            [1] { $count ->
                [one] { NUMBER($count, useGrouping: 0) } reference now points to this block
               *[other] { NUMBER($count, useGrouping: 0) } references now point to this block
            }
           *[other] { $count ->
                [one] { NUMBER($count, useGrouping: 0) } reference is now broken
               *[other] { NUMBER($count, useGrouping: 0) } references are now broken
            }
        }
    }

page-props-stale-panel =
    { $stale ->
        [graph] The graph changed while its properties panel was open, so "{ $key }" was not saved. Reopen the panel to edit it.
        [page] This page changed or was reloaded while its properties panel was open, so "{ $key }" was not saved. Reopen the panel to edit it.
       *[block] This block changed or was reloaded while its properties panel was open, so "{ $key }" was not saved. Reopen the panel to edit it.
    }
graph-rewrite-conflicted-referrers =
    { $count ->
        [one] One page still mid-merge keeps its references to “{ $page }”: { $files }. Resolve the merge, then update them.
       *[other] { NUMBER($count, useGrouping: 0) } pages still mid-merge keep their references to “{ $page }”: { $files }. Resolve the merge, then update them.
    }
block-link-copy-save-failed =
    { $kind ->
        [ref] Couldn't save the block id — reference not copied.
       *[embed] Couldn't save the block id — embed not copied.
    }
block-link-copy-copied =
    { $kind ->
        [ref] { $count ->
            [one] Copied block ref
           *[other] Copied block refs
        }
       *[embed] { $count ->
            [one] Copied block embed
           *[other] Copied block embeds
        }
    }
