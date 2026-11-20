## All Pages and its sidebar links
pages-all-title = All pages
page-icon-set = Set page icon…
page-icon-remove = Remove page icon
page-icon-label = Page icon
page-icon-save = Save icon
page-icon-hint = Enter an emoji or short text. Windows: Win + .; macOS: Control + Command + Space.
page-icon-stale = This page changed; reopen the page actions menu to edit its icon.
pages-all-filter = Filter page names
pages-all-show-journals = Show journals
pages-all-loading = Loading…
pages-all-column-page = Page
pages-all-column-updated = Updated
pages-all-column-journal-date = Journal date
pages-all-no-matches = No page names match.
pages-all-empty = No pages yet.
# $count (number): listed pages.
pages-all-count = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } page
   *[other] { NUMBER($count, useGrouping: 0) } pages
    }
# $count (number): matching pages; $total (number): all listed pages.
pages-all-filtered-count = { NUMBER($count, useGrouping: 0) } of { NUMBER($total, useGrouping: 0) } pages
config-query-title = Query
config-query-unsupported = Tine can't run this query from config.edn yet.
config-query-details = Technical details

## Namespaces view (GH #335)
sidebar-namespaces-title = Namespaces
pages-namespaces-open-full-view = Open the full view…
pages-namespaces-filter = Filter namespaces and pages
pages-namespaces-expand-all = Expand all
pages-namespaces-collapse-all = Collapse all
pages-namespaces-column-namespace = Namespace
pages-namespaces-column-pages = Pages
pages-namespaces-no-matches = No namespace or page name matches.
pages-namespaces-empty = No namespaced pages yet. Name a page like Project/Roadmap to start one.
# $name (text): the full path of the namespace.
pages-namespaces-expand = Expand { $name }
pages-namespaces-collapse = Collapse { $name }
# $count (number): entries whose path matches the filter.
pages-namespaces-matches = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } match
   *[other] { NUMBER($count, useGrouping: 0) } matches
    }
# $pages (number): pages that have a namespaced name; $namespaces (number): levels that have something below them.
pages-namespaces-count = { $pages ->
    [one] { NUMBER($pages, useGrouping: 0) } page
   *[other] { NUMBER($pages, useGrouping: 0) } pages
    } in { $namespaces ->
    [one] { NUMBER($namespaces, useGrouping: 0) } namespace
   *[other] { NUMBER($namespaces, useGrouping: 0) } namespaces
    }
rename-refused-own-child = Couldn't rename “{ $old }” to “{ $new }”: “{ $new }” is already a page or an alias inside “{ $old }”, and every page under “{ $old }” moves with it.
rename-refused-file-taken = Couldn't rename: the file { $file } belongs to the page “{ $page }”. Rename or merge that page first.
rename-refused-not-a-page = Couldn't rename “{ $name }”: no page or alias has that name.
rename-refused-day = Couldn't rename “{ $name }”: it is a journal day, and days keep their names.
rename-refused-alias-held = Couldn't rename: “{ $alias }” is an alias of the page “{ $page }”. Pick another name, or remove that alias from “{ $page }” first.
rename-refused-conflict-markers = Couldn't rename: { $file } has unresolved conflict markers and would need changes to move. Resolve the conflict first.
rename-refused-capture = Couldn't rename: “{ $name }” would become “{ $image }”, which is already the page “{ $page }”. Rename or merge that page first.
rename-refused-contested = Couldn't rename: several files claim “{ $name }” ({ $files }). Merge one into the other or remove one first.
rename-refused-stale-merge = Cannot merge: { $file } no longer holds the name you chose. Start the rename again to choose the remaining page.
rename-refused-unindexed = Cannot rename or merge: Tine cannot read { $file } safely. Repair its encoding or simplify the file in another editor, then try again.
rename-refused-read-only-org = Cannot rename or merge: Tine cannot save { $file } without changing its content. Edit it in another Org editor or move the notes into a new page, then try again.
rename-refused-different-formats = Cannot merge: { $source } and { $target } use different formats. Move the notes into pages with the same format, then try again.
rename-refused-merge-conflict-markers = Cannot merge: Resolve the conflict markers in { $file }, then try again. Nothing was merged.
rename-refused-same-file = Cannot merge: { $file } is both the source and the remaining file. Choose two different files.
rename-refused-stale-source = Cannot rename: { $file } no longer identifies the page you chose. Reopen the page and start the rename again.
rename-refused-unreadable = Cannot merge: Tine could not read { $file }. Close any program that holds it open, then try again. Nothing was merged.
rename-refused-title-identity = Cannot rename or merge: changing the references inside the title of { $file } would give the page a different name. Edit that title so its references do not name the page being renamed, then try again. Nothing was renamed or merged.
rename-refused-body-layout = Cannot merge: Tine cannot preserve the block structure of { $file } when joining the files. Align the top-level blocks with the left edge in that file, save it and try again. Nothing was merged.
rename-refused-indented-body = Cannot merge: the first block of { $file } is indented, so Tine cannot keep its blocks at the top level when joining the files. Align the top-level blocks with the left edge in that file, save it and try again. Nothing was merged.
rename-refused-directive-escape = Cannot merge: { $file } contains a #+END_QUOTE line outside a quote, which would end the merged page's quote early. Move that line into a quote or remove it in that file, then try again. Nothing was merged.
rename-refused-alias-spelling = Cannot merge: Tine cannot move the alias declaration “{ $alias }” in { $file } without losing its spelling. Replace that declaration with plain alias names separated by commas in that file, save it and try again. Nothing was merged.
rename-refused-duplicate-files = Cannot rename: these files claim the same page: { $files }. Merge one file into the other or remove one, then try again.
# The rename dialog when the typed name is only another page's alias (D-20261115-HELP).
rename-alias-dialog-title = “{ $to }” already opens “{ $owner }”
rename-alias-dialog-intro = “{ $to }” is an alias, so renaming “{ $from }” to it is not allowed as it stands. These pages take part:
# The page's alias line as the file has it (graph syntax, kept as written).
rename-alias-dialog-aliases = alias:: { $aliases }
rename-alias-dialog-opens-now = opens “{ $to }” now
rename-alias-dialog-this-page = the page you are renaming
rename-alias-dialog-merge = Merge “{ $from }” into “{ $owner }”
rename-alias-dialog-merge-body = The blocks of “{ $from }” move to the end of “{ $owner }”, its aliases join those of “{ $owner }”, and links to “{ $from }” are changed to “{ $owner }”.
rename-alias-dialog-tags = The tags of both pages are united first: tags:: { $united }
rename-alias-dialog-merge-unavailable = Merging into “{ $owner }” is not possible: { $reason }
rename-alias-dialog-own = Use the name “{ $to }” for this page
rename-alias-dialog-own-edit = First, “{ $to }” is removed from the aliases of “{ $page }”: { $key }:: { $before } becomes { $key }:: { $after }
rename-alias-dialog-own-edit-line = First, the line { $key }:: { $before } is removed from “{ $page }”.
rename-alias-dialog-own-body = Then “{ $from }” is renamed to “{ $to }”.
rename-alias-dialog-own-unavailable = Using “{ $to }” for this page is not possible: { $reason }
rename-alias-dialog-alias-line = the alias line of “{ $page }” holds more than the aliases, or repeats. Remove “{ $to }” from it yourself, then rename again.
rename-alias-dialog-other = Tine could not plan it. Start the rename again.
rename-alias-dialog-none = Pick another name instead.
rename-alias-dialog-pages = { $count ->
    [one] Changes 1 page: { $pages }
   *[other] Changes { NUMBER($count, useGrouping: 0) } pages: { $pages }
    }
rename-alias-dialog-retarget = { $count ->
    [one] Links to “{ $name }” open “{ $after }” instead of “{ $before }”, their text unchanged, in 1 page: { $pages }
   *[other] Links to “{ $name }” open “{ $after }” instead of “{ $before }”, their text unchanged, in { NUMBER($count, useGrouping: 0) } pages: { $pages }
    }
rename-alias-dialog-more = { $pages } and { NUMBER($more, useGrouping: 0) } more
rename-alias-dialog-stale-what = The rename choice
# A rename helper's own edits (D-20261116-TAGH, D-20261115-HELP).
rename-helper-page-changed = Couldn't rename: “{ $page }” changed since the rename was planned. Nothing was renamed. Start the rename again.
rename-helper-unsaved = Couldn't rename: the change to “{ $page }” could not be saved, so it was put back. Nothing was renamed.
rename-helper-not-restored = The rename did not happen, and Tine could not save the restored text of { $pages }. Check those pages.
# The day move and the contested-name dialog (D-20261111-CMDF, D-20261019-M2(6), D-20261115-HELP).
rename-day-dialog-title = “{ $day }” keeps its date. Move its notes to “{ $name }”?
rename-day-dialog-new = Its blocks move to a new page, “{ $name }”.
rename-day-dialog-into = Its blocks move into the existing page “{ $name }”, after that page's own blocks.
rename-day-dialog-keeps = “{ $day }” keeps its page properties and gets one block: Moved to [[{ $name }]]
rename-day-dialog-alias = Its alias line stays on the day: alias:: { $aliases }
rename-day-dialog-references = { $count ->
    [one] Links to “{ $day }” keep opening the day, in 1 page: { $pages }
   *[other] Links to “{ $day }” keep opening the day, in { NUMBER($count, useGrouping: 0) } pages: { $pages }
    }
rename-day-dialog-no-references = No page links to “{ $day }”.
rename-day-dialog-move = Move the notes to “{ $name }”
rename-day-dialog-move-into = Move the notes into “{ $name }”
rename-day-moved = Moved the notes of “{ $day }” to “{ $name }”.
rename-day-nothing = “{ $day }” has no notes to move.
rename-day-not-moved = The notes of “{ $day }” were not moved. Nothing changed.
rename-contested-dialog-title = Several files claim “{ $name }”
rename-contested-dialog-intro = “{ $from }” cannot be renamed to “{ $to }” while they do. Keep one file and merge the others into it: their blocks follow its own, their aliases join it, and the other files go to the trash. No link changes. Then the rename is tried again.
rename-contested-dialog-keep = Keep { $file }, merge the others into it
