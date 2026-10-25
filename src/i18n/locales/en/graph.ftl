
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
root-recovery-notice = Unsaved edits from { $location } are still on this device. The graph may be moved or unavailable (for example, a disconnected drive). Recover only if these edits belong in this graph.
root-recovery-earlier-location = an unknown graph location
root-recovery-recover = Recover into this graph
root-recovery-not-now = Not now
root-recovery-current-unsaved = Save or resolve this graph's unsaved edits first.
root-recovery-failed = Could not recover the unsaved edits: { $error }
root-moved-notice = The graph folder was moved, replaced or disconnected while open. Your unsaved edits are kept on this device. Close this window, then reopen the graph from its new location.
backup-old-location = Made when the graph was at { $path }. That graph may be moved or unavailable (for example, a disconnected drive).
backup-delete-old = Delete old-location backups
backup-delete-old-confirm = Delete the backups made when the graph was at { $path }? The graph may be moved or unavailable (for example, a disconnected drive). These may be the only backups of a graph on a disconnected drive. This removes those local snapshots from this device.
backup-delete-old-failed = Could not delete old-location backups: { $error }
root-recovery-owner-held = These unsaved edits are still held by another Tine window or process, or by a host that is finishing its save. Close that graph and wait for it to finish, then try again.
root-recovery-unavailable = These unsaved edits are no longer available in device recovery. Reopen the graph to check again.
root-recovery-lock-unavailable = Tine could not reserve this graph for recovery. Close and reopen the graph, then try again.
root-recovery-moved-durability-unwitnessed = The unsaved edits were moved into this graph, but Tine could not confirm the move's durability. Keep this window open and check the recovered edits before closing Tine.
graph-open-other-app = The graph is open in another Tine window.
root-moved-draft-error = The graph folder was moved, replaced or disconnected, and its recovery copy could not be written. Keep this window open and copy your unsaved text until device storage is working again.
