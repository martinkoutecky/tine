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
rename-alias-choice-title = “{ $to }” is another page's alias
rename-alias-choice-body = “{ $to }” is an alias of “{ $owner }”. Merge “{ $from }” into “{ $owner }”, or make “{ $to }” a page of its own? A page of its own stays an alias of “{ $owner }” too, so links to “{ $to }” still show in the Linked References of “{ $owner }”.
rename-alias-choice-fileless = “{ $to }” is an alias of “{ $owner }”. Merge references to “{ $from }” into “{ $owner }”? “{ $from }” has no file to make into a page of its own.
rename-alias-choice-no-merge = Merging is not offered: “{ $owner }” is inside “{ $from }” and would move with it.
rename-alias-choice-merge = Merge into “{ $owner }”
rename-alias-choice-own-page = Make “{ $to }” its own page (still an alias of “{ $owner }”)
rename-refused-own-child = Couldn't rename “{ $old }” to “{ $new }”: “{ $new }” is already a page or an alias inside “{ $old }”, and every page under “{ $old }” moves with it.
rename-refused-file-taken = Couldn't rename: the file { $file } belongs to the page “{ $page }”. Rename or merge that page first.
rename-refused-not-a-page = Couldn't rename “{ $name }”: no page or alias has that name.
rename-refused-day = Couldn't rename “{ $name }”: it is a journal day, and days keep their names.
rename-refused-alias-held = Couldn't rename: “{ $alias }” is an alias of the page “{ $page }”. Pick another name, or remove that alias from “{ $page }” first.
rename-refused-conflict-markers = Couldn't rename: { $file } has unresolved conflict markers and would need changes to move. Resolve the conflict first.
rename-refused-capture = Couldn't rename: “{ $name }” would become “{ $image }”, which is already the page “{ $page }”. Rename or merge that page first.
rename-refused-contested = Couldn't rename: several files claim “{ $name }” ({ $files }). Merge one into the other or remove one first.
rename-alias-choice-stale-what = The rename
rename-refused-stale-merge = Cannot merge: { $file } no longer holds the name you chose. Start the rename again to choose the remaining page.
rename-refused-unindexed = Cannot rename or merge: Tine cannot read { $file } safely. Repair its encoding or simplify the file in another editor, then try again.
rename-refused-read-only-org = Cannot rename or merge: Tine cannot save { $file } without changing its content. Edit it in another Org editor or move the notes into a new page, then try again.
rename-refused-different-formats = Cannot merge: { $source } and { $target } use different formats. Move the notes into pages with the same format, then try again.
rename-refused-merge-conflict-markers = Cannot merge: Resolve the conflict markers in { $file }, then try again. Nothing was merged.
rename-refused-same-file = Cannot merge: { $file } is both the source and the remaining file. Choose two different files.
rename-refused-stale-source = Cannot rename: { $file } no longer identifies the page you chose. Reopen the page and start the rename again.
rename-refused-title-identity = Cannot rename or merge: changing the references inside the title of { $file } would give the page a different name. Edit that title so its references do not name the page being renamed, then try again. Nothing was renamed or merged.
rename-refused-body-layout = Cannot merge: Tine cannot preserve the block structure of { $file } when joining the files. Align the top-level blocks with the left edge in that file, save it and try again. Nothing was merged.
rename-refused-indented-body = Cannot merge: the first block of { $file } is indented, so Tine cannot keep its blocks at the top level when joining the files. Align the top-level blocks with the left edge in that file, save it and try again. Nothing was merged.
rename-refused-directive-escape = Cannot merge: { $file } contains a #+END_QUOTE line outside a quote, which would end the merged page's quote early. Move that line into a quote or remove it in that file, then try again. Nothing was merged.
rename-refused-alias-spelling = Cannot merge: Tine cannot move the alias declaration “{ $alias }” in { $file } without losing its spelling. Replace that declaration with plain alias names separated by commas in that file, save it and try again. Nothing was merged.
rename-refused-duplicate-files = Cannot rename: these files claim the same page: { $files }. Merge one file into the other or remove one, then try again.
