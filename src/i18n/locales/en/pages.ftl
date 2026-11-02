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
pages-all-open-full-list = Open the full list…

# $count (number): listed pages.
pages-all-count = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } page
   *[other] { NUMBER($count, useGrouping: 0) } pages
    }
# $count (number): matching pages; $total (number): all listed pages.
pages-all-filtered-count = { NUMBER($count, useGrouping: 0) } of { NUMBER($total, useGrouping: 0) } pages
# $count (number): pages beyond the sidebar's visible limit.
pages-all-open-more = +{ NUMBER($count, useGrouping: 0) } more — open the full list…
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
