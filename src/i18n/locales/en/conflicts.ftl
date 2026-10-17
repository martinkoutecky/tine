# Conflict review: unsaved drafts, on-disk changes, sync copies and duplicate journal files.
# Graph/page names and file paths in variables remain verbatim.
conflict-bar-review = Review

conflict-bar-use-disk-version = Use disk version

conflict-bar-keep-mine-overwrite = Keep mine (overwrite)

conflict-overview-unsaved-drafts = Unsaved drafts

conflict-overview-sync-conflict-copies = Sync conflict copies

conflict-overview-version-control-merge-markers = Version-control merge markers

conflict-overview-duplicate-journal-days = Duplicate journal days

conflict-overview-conflicts = Conflicts

conflict-overview-no-conflicts-pages-that-need-a-decision-appear = No conflicts. Pages that need a decision appear here.

conflict-overview-each-page-is-resolved-on-the-page-itself = Each page is resolved on the page itself, block by block. Shift-click opens it in the right sidebar so this list stays in view.

conflict-overview-open-the-page-to-resolve-it-shift-click = Open the page to resolve it (shift-click: right sidebar)

conflict-overview-discard-copy = Discard copy

conflict-overview-conflict-copy = conflict copy

conflict-resolution-previous-conflict = Previous conflict

conflict-resolution-next-conflict = Next conflict

conflict-resolution-this-file-still-contains-merge-markers-so-tine = This file still contains merge markers, so Tine refuses to save it: rewriting it would re-indent the markers and silently lose one side. Choose below and apply to make it an ordinary page again.

conflict-resolution-choosing-below-folds-the-other-file-into-this = Choosing below folds the other file into this day and moves it to the recoverable trash. Or act on a file directly:

conflict-resolution-reading-both-versions = Reading both versions…

conflict-resolution-these-two-files-can-t-be-folded-together = These two files can’t be folded together: one is Markdown and the other Org. Use the file actions above.

conflict-resolution-couldn-t-read-this-conflict = Couldn’t read this conflict. ({ $error })

conflict-resolution-conflict-unavailable = Couldn’t read this conflict.

conflict-resolution-the-two-versions-are-identical-nothing-to-decide = The two versions are identical — nothing to decide.

conflict-resolution-the-copy-is-safe-to-discard-from-the = { " " }The copy is safe to discard from the Conflicts overview.

conflict-resolution-the-other-file-is-safe-to-trash-above = { " " }The other file is safe to trash above.

conflict-resolution-use-disk-version-above-loses-nothing = { " " }“Use disk version” above loses nothing.

conflict-resolution-nothing-was-pre-selected-no-common-version-is = Nothing was pre-selected — no common version is known, so both sides are kept.

conflict-resolution-re-applies-tine-s-own-suggestions-a-merge = Re-applies Tine's own suggestions. A merge tool's proposed text keeps your current choice.

conflict-resolution-apply-all-suggested = Apply all suggested

conflict-resolution-keep-both-everywhere = Keep both everywhere

conflict-resolution-show-unchanged = show unchanged

conflict-resolution-the-page-s-own-properties-differ-keep = The page’s own properties differ. Keep

conflict-resolution-both-merge = both (merge)

conflict-resolution-applying-writes-the-merged-page-without-any-markers = Applying writes the merged page without any markers; the file as it was moves to the recoverable trash.

conflict-resolution-applying-writes-the-merged-page-only-if-the = Applying writes the merged page only if the file is still the version shown; a newer change refreshes this comparison.

conflict-resolution-the-other-file-moves-to-the-recoverable-trash = The other file moves to the recoverable trash once this is applied, leaving the day one file.

conflict-resolution-the-copy-moves-to-the-recoverable-trash-once = The copy moves to the recoverable trash once this is applied.

conflict-resolution-applying = Applying…

conflict-resolution-apply-resolution = Apply resolution

diff-rows-earlier-lines-are-the-same-on-both-sides = Earlier lines are the same on both sides

diff-rows-merged = Merged

diff-rows-unchanged = unchanged

diff-rows-pre-selected-from-the-last-version-both-sides = Pre-selected from the last version both sides agreed on

diff-rows-suggested = suggested

diff-rows-hide-the-full-bodies = Hide the full bodies

diff-rows-show-all-lines = Show all { $line_count } lines

diff-rows-merged-tool = Merged (tool)

journal-conflict-file-row-show-this-file-s-contents = Show this file's contents

journal-conflict-file-row-canonical = { " " }· canonical

journal-conflict-file-row-open-this-exact-file-editable = Open this exact file (editable)

journal-conflict-file-row-open = Open

journal-conflict-file-row-append-this-file-s-blocks-to-the-canonical = Append this file's blocks to the canonical day, then trash it

journal-conflict-file-row-merge = Merge

journal-conflict-file-row-move-this-file-to-a-uniquely-named-page = Move this file to a uniquely-named page

journal-conflict-file-row-rename = Rename…

journal-conflict-file-row-trash = Trash

# $error is an untranslated diagnostic from the operating system or backend.
journal-conflict-file-row-couldn-t-read-this-file = Couldn't read this file: { $error }

journal-conflict-file-row-new-page-name = New page name

journal-conflict-file-row-save = Save

journal-conflict-file-row-cancel = Cancel

journal-conflict-file-row-empty-file = (empty file)

journal-filename-panel-journal-files-named-by-title = Journal files named by title

journal-filename-panel-these-journal-files-aren-t-named-after-their = These journal files aren’t named after their date, so Tine can’t place them in the journal feed and their days look empty. Renaming them fixes that, but it changes files you own, so Tine never does it on its own. Version control and sync will see these as renames.

journal-filename-panel-renaming = Renaming…

journal-filename-panel-rename-to-date-names = Rename to date names

live-conflicts-your-unsaved-edits = Your unsaved edits

live-conflicts-the-file-on-disk-now = The file on disk now

mock-conflicts-this-device = This device

conflict-bar-live-notice = <strong>“{ $name }” changed on disk while you were editing it.</strong> Your edits are kept; review them against the disk version on the page.
conflict-bar-unsaved-notice =
    { $deleted ->
        [1] <strong>“{ $name }” was deleted on disk</strong>. Your unsaved changes weren't written.
       *[other] <strong>“{ $name }” changed on disk</strong>. Your unsaved changes weren't written.
    }
conflict-overview-orphan-source = sync copy · { $tag } · its page no longer exists
conflict-overview-discard-confirmation = Discard the conflict copy “{ $name }”?{ "\u000A\u000A" }It moves to logseq/.tine-trash (recoverable). The current “{ $page }” is left as-is.
conflict-resolution-trash-confirmation = Move the journal file “{ $name }” to the trash?{ "\u000A\u000A" }It's a duplicate of another file for the same day. It moves to logseq/.tine-trash (recoverable).
conflict-resolution-count =
    { $count ->
        [one] { NUMBER($count, useGrouping: 0) } conflict
       *[other] { NUMBER($count, useGrouping: 0) } conflicts
    }
conflict-resolution-base-suggestions = { $base } (used for the suggestions)
conflict-resolution-suggested-summary = { $suggested } of { $total } pre-selected from the last version both sides agreed on — review and confirm.
conflict-resolution-keep-side = Keep { $side }
conflict-resolution-review-count = { $count } to review

journal-filename-panel-confirmation =
    { $count ->
        [one] Rename { NUMBER($count, useGrouping: 0) } journal file to their date names?{ "\u000A\u000A" }A snapshot is taken first, so the original names stay in Backups & recovery. Nothing is overwritten: a file whose date name is taken is left alone.
       *[other] Rename { NUMBER($count, useGrouping: 0) } journal files to their date names?{ "\u000A\u000A" }A snapshot is taken first, so the original names stay in Backups & recovery. Nothing is overwritten: a file whose date name is taken is left alone.
    }

# Helper-return and component-prop interface text (GH #107).
conflicts-prose-unsaved-draft = unsaved draft
conflicts-prose-mine = mine
conflicts-prose-theirs = theirs
conflicts-prose-mine-short = Mine
conflicts-prose-theirs-short = Theirs
conflicts-prose-unresolved-merge-from-your-version-control-tool = Unresolved merge from your version-control tool
conflicts-prose-your-edits-and-a-newer-version-on-disk = Your edits and a newer version on disk
conflicts-prose-this-day-has-more-than-one-file = This day has more than one file
conflicts-prose-two-versions-of-this-page-arrived = Two versions of this page arrived
conflicts-prose-proposed-by-your-merge-tool-s-suggested-resolution-still-applied = Proposed by your merge tool's suggested resolution — still applied only when you confirm.
conflicts-prose-both-edits-combined-offered-because-they-touch-different-parts-of = Both edits combined — offered because they touch different parts of the same body
conflicts-prose-both = Both
conflicts-prose-merged = Merged
conflicts-prose-keep = Keep
conflicts-prose-drop = Drop
conflicts-prose-skip = Skip
conflicts-prose-pull-in = Pull in

conflicts-prose-day-files = { NUMBER($count, useGrouping: 0) } files for one day
conflicts-prose-sync-copy = sync copy · { $side }
conflicts-prose-merge-sides = merge markers · { $mine } vs { $theirs }
conflicts-prose-block-count = { NUMBER($count, useGrouping: 0) } { $count ->
        [one] block
       *[other] blocks
    }

conflicts-prose-copy-fallback = conflict copy
conflicts-prose-local-fallback = local
conflicts-prose-incoming-fallback = merged-in
conflicts-prose-copy-month = { $month ->
        [1] Jan
        [2] Feb
        [3] Mar
        [4] Apr
        [5] May
        [6] Jun
        [7] Jul
        [8] Aug
        [9] Sep
        [10] Oct
        [11] Nov
        [12] Dec
       *[other] { $month }
    }
conflicts-prose-dated-copy = Sync copy · { $month } { $day }{ $year ->
        [current] { "" }
       *[other] { " " }{ $year }
    }

conflicts-prose-local-side = Local side
conflicts-prose-merged-side = Merged-in side
conflicts-prose-conflict-copy = Conflict copy
