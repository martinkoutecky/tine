# Query export and sheet controls.
# Query/property names and code displayed in variables remain verbatim.
query-export-dialog-export-query-results = Export query results

query-export-dialog-name = Name

query-export-dialog-resolving-pages = Resolving pages…

query-export-dialog-journal = { " " }· journal

query-export-dialog-all-blocks-on-these-pages-will-be-exported = All blocks on these pages will be exported, including blocks that did not match.

query-export-dialog-destination = Destination

query-export-dialog-replace-it-the-previous-export-is-kept-in = { " " }Replace it (the previous export is kept in recovery)

query-export-dialog-adjust-limit-in-settings = Adjust limit in Settings…

query-export-dialog-cancel = Cancel

query-export-dialog-exporting = Exporting…

query-export-dialog-export = Export

query-export-limit-setting-query-export-size-limit = Query export size limit

query-export-limit-setting-query-export-size-limit-mi-b = Query export size limit (MiB)

query-export-limit-setting-default-1-gi-b = Default: 1 GiB

query-export-limit-setting-mi-b = { " " }MiB

query-export-limit-setting-reset = Reset

query-export-limit-setting-default-1-gi-b-copied-assets-must-fit = Default: 1 GiB. Copied assets must fit this limit; an oversized export stops. Saved on this device.

query-legacy-table-content = Content

query-legacy-table-page = Page

query-listbox-type-to-filter = Type to filter

query-page-results-page = page

query-page-results-journal = Journal

query-page-results-page-label = Page

query-page-results-page-results = Page results

query-page-results-page-results-grouped = Page results grouped

query-page-results-no-value = No value

query-result-sections-pages = Pages

query-result-sections-blocks = Blocks

query-result-sections-searching = Searching…

query-result-sections-no-matching-pages = No matching pages.

query-result-sections-no-matching-blocks = No matching blocks.

query-vocabulary-picker-condition-field = Condition field

query-vocabulary-picker-search-fields-and-properties = Search fields and properties

query-vocabulary-picker-reading-this-graph-s-properties = Reading this graph's properties…

query-vocabulary-picker-try-again = Try again

query-vocabulary-picker-declared = { $declared } (declared)

sheet-aggregate-footer-hide-aggregate-footer = Hide aggregate footer

sheet-aggregate-footer-show-aggregate-footer = Show aggregate footer

sheet-aggregate-footer-add-aggregate = Add aggregate

sheet-aggregate-footer-change-aggregate = Change aggregate ({ $active_label })

sheet-grid-show-previous-rows = Show previous rows

sheet-grid-show-previous-columns = Show previous columns

sheet-grid-add-column = Add column

sheet-grid-add-row = Add row

sheet-grid-cell-menu = Cell menu

crossing-notice-warning =
    { $has_feature ->
        [1] This query now uses Tine features Logseq can't read (<code>{ $feature }</code>). Logseq will show the block as plain text.
       *[other] This query now uses Tine features Logseq can't read. Logseq will show the block as plain text.
    }
crossing-notice-changed-block = The block now reads: <code>{ $text }</code>
query-export-dialog-summary = { $results } results on { $pages } complete pages
query-export-dialog-existing = An export named <code>{ $folder }</code> already exists.
query-export-dialog-separate = Create a separate export as <code>{ $folder }</code>
query-export-dialog-path = Destination: <code>{ $path }</code>. This exports these pages regardless of their public setting as a static site and read-only app. It is not uploaded. Unsaved edits are not included.
query-page-results-alias = matched alias { $alias }
query-result-sections-more-matches =
    { $kind ->
        [page] More pages match than are shown.
       *[other] More blocks match than are shown.
    }
query-vocabulary-picker-registry-failed = This graph's properties could not be read. { $detail }
property-type-mismatches = { $count } blocks or pages do not match
sheet-grid-more-rows =
    { $load ->
        [1] Load more rows ({ $start }-{ $end } of { $total })
       *[other] Show next rows ({ $start }-{ $end } of { $total })
    }
sheet-grid-more-columns =
    { $load ->
        [1] Load more columns ({ $start }-{ $end } of { $total })
       *[other] Show next columns ({ $start }-{ $end } of { $total })
    }

query-vocabulary-picker-use-property =
    { $unit ->
        [pages] Use "{ $key }" as a page property
       *[other] Use "{ $key }" as a block property
    }
