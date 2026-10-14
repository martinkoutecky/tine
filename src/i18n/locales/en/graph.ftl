
# Interface text; variables carry untranslated names, paths or diagnostic details.
graph-retry = Retry
graph-restart-tine = Restart Tine
graph-this-graph-s-assets-folder-points-outside-the = This graph's assets folder points outside the graph to:{ "\u000A" }{ "\u000A" }{ $assetsDirectory }{ "\u000A" }{ "\u000A" }Allow Tine to read and write assets in this directory? This approval is stored only on this device.
graph-allow-external-assets-directory = Allow external assets directory?
graph-graph-not-opened-its-external-assets-directory = Graph not opened: its external assets directory was not approved ({ $assetsDirectory }).
graph-some-pages-couldn-t-be-saved-resolve = Some pages couldn't be saved — resolve conflicts before switching graphs.
# $error is an untranslated diagnostic from the operating system or backend.
graph-couldn-t-prepare-the-icloud-graph = Couldn't prepare the iCloud graph. ({ $error })
graph-tineoutline-can-only-open-folders-inside-on = TineOutline can only open folders inside On My iPhone → TineOutline or iCloud Drive → TineOutline.
graph-pdf-changes-couldn-t-be-saved-the = PDF changes couldn't be saved — the current graph is still open.
graph-a-rename-is-still-running-switch = A rename is still running — switch graphs once it finishes.
graph-waiting-for-tine-to-finish-saving = Waiting for Tine to finish saving “{ $saving }” before opening this graph…
# English retains the existing "(s)" wording; translations may inflect by count.
graph-recovered-unsaved-edits-on-page-s-tine = { $count ->
    [one] Recovered unsaved edits on { $count } page(s) — Tine is saving them: { $pages }.
   *[other] Recovered unsaved edits on { $count } page(s) — Tine is saving them: { $pages }.
    }
# English retains the existing "(s)" wording; translations may inflect by count.
graph-page-s-still-have-edits-that-are = { $count ->
    [one] { $count } page(s) still have edits that are not on disk; Tine keeps saving them: { $pages }.
   *[other] { $count } page(s) still have edits that are not on disk; Tine keeps saving them: { $pages }.
    }
# English retains the existing "(s)" wording; translations may inflect by count.
graph-tine-left-unreadable-crash-recovery-file-s-in-place = { $count ->
    [one] Tine left { $count } unreadable crash-recovery file(s) in place: { $files }.
   *[other] Tine left { $count } unreadable crash-recovery file(s) in place: { $files }.
    }
graph-crash-recovery-is-off-for-this-graph = Crash recovery is off for this graph: { $unavailable }. Your edits still save.
graph-crash-recovery-is-on-again = Crash recovery is on again.
# $error is an untranslated diagnostic from the operating system or backend.
graph-crash-recovery-is-still-off = Crash recovery is still off: { $error }.
graph-crash-recovery-copies-from-an-older-tine-are = Crash-recovery copies from an older Tine are kept untouched in { $file }.
graph-page-already-exists-merge-into = Page “{ $to }” already exists. Merge “{ $from }” into it?
# $error is an untranslated diagnostic from the operating system or backend.
graph-couldn-t-open-the-folder-picker = Couldn't open the { $platform } folder picker. ({ $error })
graph-tine-needs-a-folder-on-the-device-s = Tine needs a folder on the device's own storage. This provider folder can't be opened directly. Use your provider's sync-to-local-folder feature, then pick that local folder in Open graph.
graph-grant-all-files-access-for-tine-then = Grant "All files access" for Tine, then tap Open again.
graph-choose-a-folder-inside-on-my-iphone = Choose a folder inside On My iPhone → TineOutline or iCloud Drive → TineOutline. Other apps' folders and Files providers aren't supported yet.
# $error is an untranslated diagnostic from the operating system or backend.
graph-couldn-t-prepare-the-icloud-location = Couldn't prepare the iCloud location. ({ $error })
graph-tineoutline-can-only-create-graphs-inside-on = TineOutline can only create graphs inside On My iPhone → TineOutline or iCloud Drive → TineOutline.
# $graphState is open when an existing graph remains open, otherwise none. $error is an untranslated diagnostic.
graph-couldn-t-create-the-graph = { $graphState ->
    [open] Couldn't create the graph. The current graph is still open. ({ $error })
   *[none] Couldn't create the graph. ({ $error })
    }
graph-open-graph = Open graph
graph-created-the-graph-at-but-couldn-t = Created the graph at { $root }, but couldn't open it. ({ $detail })
graph-created-the-graph-at-but-it = Created the graph at { $root }, but it wasn't opened. You can open this folder again.
graph-created-the-graph-at-and-focused-its = Created the graph at { $root } and focused its existing window.
graph-created-the-graph-at-it-is = Created the graph at { $root }. It is already open.
graph-created-and-opened-the-graph-at = Created and opened the graph at { $root }.
