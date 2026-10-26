# Query export and sheet controls.
# Complete sentences and selectors for the query conversion lane.
query-text-vocabulary-guide = The fields and properties this graph has are in the picker above; the query language is in the Guide under <em>Find and revisit → Query text (TQL)</em>.
query-registry-failed = This graph's properties could not be read. { $detail }
query-anchor-unchanged = The anchor wasn't changed: { $detail }
query-grouped-by = Grouped by { $field }
query-skipped-non-numeric = { " " }({ NUMBER($count, useGrouping: 0) } non-numeric skipped)
query-skipped = { " " }({ NUMBER($count, useGrouping: 0) } skipped)
# These code spans are literal query syntax.
query-og-bare-task = <code>{ $syntax }</code> without markers adds no condition, as in Logseq.{ " " }
query-og-bare-priority = <code>{ $syntax }</code> without levels adds no condition, as in Logseq.{ " " }
query-preview-load-failed = Results couldn't be loaded: { $detail }
query-preview-count =
    { $kind ->
        [page] { $count ->
            [one] { NUMBER($count, useGrouping: 0) } page
           *[other] { NUMBER($count, useGrouping: 0) } pages
        }
       *[other] { $count ->
            [one] { NUMBER($count, useGrouping: 0) } result
           *[other] { NUMBER($count, useGrouping: 0) } results
        }
    }
query-preview-limit = { " " }(showing the first { NUMBER($count, useGrouping: 0) })
query-preview-families =
    { $pages ->
        [one] { NUMBER($pages, useGrouping: 0) } page
       *[other] { NUMBER($pages, useGrouping: 0) } pages
    } · { $blocks ->
        [one] { NUMBER($blocks, useGrouping: 0) } block
       *[other] { NUMBER($blocks, useGrouping: 0) } blocks
    }
query-preview-family-limit = { " " }(showing the first { NUMBER($count, useGrouping: 0) } of each)
query-preview-pages-note = Pages: { $detail }
query-preview-blocks-note = Blocks: { $detail }
query-workspace-result-count =
    { $count ->
        [one] { NUMBER($count, useGrouping: 0) } result
       *[other] { NUMBER($count, useGrouping: 0) } results
    }
query-workspace-search-failed = Search failed: { $detail }
query-workspace-explanation =
    { $hasWithout ->
        [1] { $conjunct }: { NUMBER($alone, useGrouping: 0) } alone, { NUMBER($without, useGrouping: 0) } without it
       *[other] { $conjunct }: { NUMBER($alone, useGrouping: 0) } alone
    }
query-selected-count = { NUMBER($count, useGrouping: 0) } selected
query-anchor-label =
    { $anchor ->
        [page] pages
        [both] pages and blocks
       *[other] blocks
    }
query-anchor-switch = Switching to <strong>{ query-anchor-label }</strong> — { NUMBER($count, useGrouping: 0) } of your { NUMBER($total, useGrouping: 0) } conditions don't apply to { query-anchor-label }{ $hasNames ->
        [1] { " " }(<code>{ $names }</code>)
       *[other] { "" }
    }.
query-display-change = Change
query-sort-direction = Sort { $field } { $direction ->
        [asc] descending
       *[other] ascending
    }
query-display-add-sort = + sort
query-display-add-column = + column
query-display-add-property = + property
query-display-whole-result = Whole result
# These labels accompany persisted query view/aggregate tokens; tokens never change.
query-search-positive-term = Add at least one positive search term.
query-unsupported-clause = This query clause is not supported yet: { $clause }
query-fields-regex-warning = A regular expression cannot be combined with the other friendly fields yet.
query-options-map-warning = The options map (title, collapsed) is edited with the title and Display controls, not here.
query-save-superseded = This workspace changed while it was being saved, so nothing was written. Try saving again.
query-save-enter-title = Enter a page title before saving.
query-save-enter-query = Enter a search or query before saving.
query-save-graph-changed = The graph changed before this workspace could be saved.
query-save-validation-superseded = Search validation was superseded. Try saving again.
query-save-included-term = Enter a search with at least one included term before saving.
query-save-validation-detail = Could not validate this search: { $detail }
query-save-validation-failed = Could not validate this search.
query-save-title-exists = A page named “{ $name }” already exists. Choose another title.
query-save-title-alias = “{ $name }” is an alias of an existing page. Choose another title.
query-save-local-state-changed = Could not save “{ $name }” because its local page state changed. Try again.
query-save-conflict = “{ $name }” was created or changed before this workspace could be saved. It has not been overwritten.
query-save-failed-detail = Could not save “{ $name }”: { $detail }
query-save-failed = Could not save “{ $name }”.
query-fields-quotation-warning = Quotation marks are not supported inside these fields.
query-fields-include-warning = Add at least one word or exact phrase to include.
query-dsl-empty-warning = The query DSL cannot be empty.
query-save-earlier-search = “{ $name }” was saved from the earlier search, so this workspace was left as it is.
query-workspace-view-search = Search
query-workspace-view-list = List
query-workspace-view-table = Table
query-workspace-view-board = Board

query-display-view-search = search
query-display-view-list = list
query-display-view-table = table
query-display-view-board = board
query-display-aggregate-count = count
query-display-aggregate-sum = sum
query-display-aggregate-avg = avg
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
query-export-dialog-path = Destination: <code>{ $path }</code>. This exports these pages as a static site and read-only app; pages marked public:: false are left out. It is not uploaded. Unsaved edits are not included.
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

# D-18: Logseq date windows in {{query}}.
query-date-window-refusal = Logseq ignores scheduled/deadline date windows in this query form, so this query cannot run here as written. Convert it to a Tine query to keep its date-window meaning.

query-convert = Convert

query-undo-conversion = Undo conversion

# Query and supporting controls (GH #107). Variables carry authored text or syntax verbatim.

query-query-sheet-parts-click-to-edit-this-query = Click to edit this query

query-query-sheet-parts-edit-query-filter = Edit query filter

query-query-sheet-parts-page-tag-reference = Page / tag reference

query-query-sheet-parts-task-marker = Task marker

query-query-sheet-parts-priority = Priority

query-query-sheet-parts-property = Property

query-query-sheet-parts-scheduled = Scheduled

query-query-sheet-parts-deadline = Deadline

query-query-sheet-parts-in-a-journal-page = In a journal page

query-query-sheet-parts-between-dates = Between dates

query-query-sheet-parts-full-text-search = Full-text search

query-query-sheet-parts-on-page = On page

query-query-sheet-parts-in-namespace = In namespace

query-query-sheet-parts-page-property = Page property

query-query-sheet-parts-page-tags = Page tags

query-query-sheet-parts-drag-to-reorder-or-into-another-group-or-use-the = Drag to reorder or into another group, or use the up and down arrow keys

query-query-sheet-parts-the-group-above-is-disabled-so-this-does-not-run = The group above is disabled, so this does not run

query-query-sheet-parts-disabled-kept-in-the-query-not-run = Disabled — kept in the query, not run

query-query-sheet-parts-running-press-to-disable-without-removing-it = Running — press to disable without removing it

query-query-sheet-parts-move-up = Move up

query-query-sheet-parts-move-down = Move down

query-query-sheet-parts-group-with-row-above-all-of = Group with row above — all of

query-query-sheet-parts-group-with-row-above-any-of = Group with row above — any of

query-query-sheet-parts-group-with-row-above-none-of = Group with row above — none of

query-query-sheet-parts-apply = Apply

query-query-sheet-parts-start-today-30d-2026-06-01-or-a-page = Start — today, -30d, 2026-06-01, or a page

query-query-sheet-parts-end-today-7d-2026-06-30-or-a-page = End — today, +7d, 2026-06-30, or a page

query-query-sheet-parts-page-or-tag-name = Page or tag name

query-query-sheet-parts-any-status = Any status

query-query-sheet-parts-page-name = Page name

query-query-sheet-parts-namespace-parent-page = Namespace (parent page)

query-query-sheet-parts-text-to-search-for = Text to search for

query-query-sheet-parts-search-words-and-operators = Search words and operators

query-query-sheet-parts-tag-one = Tag (one)

query-query-sheet-parts-blocks = blocks

query-query-sheet-parts-individual-bullets-anywhere-in-the-graph = individual bullets, anywhere in the graph

query-query-sheet-parts-pages = pages

query-query-sheet-parts-whole-pages = whole pages

query-query-sheet-parts-pages-and-blocks = pages and blocks

query-query-sheet-parts-pages-above-matching-blocks-below = pages above, matching blocks below

query-query-sheet-parts-find = Find

query-query-sheet-parts-what-this-query-selects = What this query selects

query-query-sheet-parts-where = where …

query-query-sheet-parts-switching-what-this-query-selects = Switching what this query selects




query-query-sheet-parts-remove-them = Remove them

query-query-sheet-parts-keep-anyway = Keep anyway

query-query-sheet-parts-cancel = Cancel

query-query-sheet-parts-selected-conditions = Selected conditions


query-query-sheet-parts-select-two-or-more-conditions-to-group-them = Select two or more conditions to group them

query-query-sheet-parts-group-selected = Group selected ▾

query-query-sheet-parts-clear = Clear

query-query-sheet-parts-all-of = All of

query-query-sheet-parts-any-of = Any of

query-query-sheet-parts-none-of = None of

query-query-sheet-parts-from = From

query-query-sheet-parts-value = Value

query-query-sheet-parts-to = To

query-query-sheet-parts-add-condition = + Add condition

query-query-sheet-parts-type-to-add-a-condition = Type to add a condition

query-query-sheet-parts-page-property-2 = { " " }(page property)

query-query-sheet-parts-reading-this-graph-s-properties = Reading this graph's properties…


query-query-sheet-parts-try-again = Try again

query-query-sheet-parts-add = Add

query-query-workspace-could-not-save = Could not save “{ $value1 }”: { $value2 }


query-query-workspace-filters-and-advanced-query = Filters and advanced query

query-query-workspace-use-the-friendly-fields-or-switch-losslessly-to-the-visual = Use the friendly fields, or switch losslessly to the visual query builder.

query-query-workspace-close-filters = Close filters

query-query-workspace-switching-back-to-friendly-fields-is-offered-only-when-it = Switching back to friendly fields is offered only when it can be lossless.

query-query-workspace-this-search-uses-a-combination-that-cannot-be-split-into = This search uses a combination that cannot be split into fields without changing it.

query-query-workspace-friendly-search-syntax = Friendly search syntax

query-query-workspace-edit-as-visual-query = Edit as visual query

query-query-workspace-all-of-these-words = All of these words

query-query-workspace-any-of-these-words = Any of these words

query-query-workspace-this-exact-phrase = This exact phrase

query-query-workspace-exclude-these-words = Exclude these words

query-query-workspace-case-sensitive-regular-expression = Case-sensitive regular expression

query-query-workspace-pattern-without = pattern without / /

query-query-workspace-couldn-t-stop-this-workspace-s-search = Couldn’t stop this workspace’s search.

query-query-workspace-search-and-query-workspace = Search and query workspace

query-query-workspace-search = Search

query-query-workspace-query-dsl = Query DSL

query-query-workspace-search-pages-and-blocks = Search pages and blocks

query-query-workspace-filters-advanced = Filters / Advanced

query-query-workspace-runs-the-saved-query-expression-against-blocks-its-presentation-is = Runs the saved query expression against blocks. Its presentation is controlled separately.

query-query-workspace-pages-match = Pages match

query-query-workspace-names-and-aliases = Names and aliases

query-query-workspace-block-content = Block content

query-query-workspace-names-or-content = Names or content

query-query-workspace-result-presentation = Result presentation

query-query-workspace-hide-explanation = Hide explanation

query-query-workspace-explain-query = Explain query

query-query-workspace-page-title = Page title

query-query-workspace-name-this-search-to-save-it-as-a-page = Name this search to save it as a page

query-query-workspace-saving = Saving…

query-query-workspace-save-page = Save page

query-query-workspace-enter-a-search-to-begin = Enter a search to begin.

query-query-workspace-enter-a-query-to-begin = Enter a query to begin.

query-query-workspace-searching = Searching…


query-query-workspace-search-superseded-by-a-newer-request = Search superseded by a newer request.


query-query-workspace-query-diagnostics = Query diagnostics

query-query-workspace-query-explanation = Query explanation

query-query-workspace-how-this-query-works = How this query works

query-query-workspace-block-results = Block results

query-query-workspace-query-results = Query results

query-query-workspace-type = Type

query-query-workspace-page = Page

query-query-workspace-content = Content

query-query-workspace-query-results-grouped-by-page = Query results grouped by page

query-query-display-move-up = Move { $value1 } up

query-query-display-move-down = Move { $value1 } down

query-query-display-remove = Remove { $value1 }

query-query-display-display-pages = Display pages

query-query-display-display = Display

query-query-display-page-display = Page display

query-query-display-block-display = Block display

query-query-display-view = View

query-query-display-query-view = Query view

query-query-display-group-by = Group by

query-query-display-no-grouping = No grouping

query-query-display-none = None

query-query-display-sort = Sort


query-query-display-asc = Asc

query-query-display-desc = Desc

query-query-display-columns = Columns

query-query-display-summarize = Summarize

query-query-display-count = + count

query-query-display-sample = Sample

query-query-display-sample-size = Sample size

query-query-display-no-limit = No limit

query-query-display-a-sample-is-a-whole-number-up-to-4294967295 = A sample is a whole number up to 4294967295.

query-query-display-no-results-sample-0 = No results (sample 0)

query-query-display-done = Done

query-query-sheet-query-filter = Query filter

query-query-sheet-try = Try:

query-query-sheet-query-conditions = Query conditions

query-query-sheet-conditions = Conditions, { $value1 }

query-query-sheet-switch-between-all-of-any-of = Switch between all of / any of

query-query-sheet-group-actions = Group actions

query-query-sheet-remove-none-of = Remove none of

query-query-sheet-ungroup = Ungroup

query-query-sheet-remove = Remove

query-query-sheet-edit-in-the-query-text-below = edit in the query text below

query-query-sheet-edit-this-part-in-the-query-text-below = Edit this part in the query text below

query-query-sheet-edit-in-the-query-text = Edit in the query text: { $value1 }

query-query-sheet-remove-condition = Remove condition

query-query-sheet-row-actions = Row actions

query-query-sheet-this-condition-was-not-understood = This condition was not understood.

query-query-sheet-did-you-mean = Did you mean

query-query-sheet-edit-as-text = Edit as text

query-query-sheet-condition-value = Condition value

query-query-result-parts-edit-properties-of = Edit properties of { $value1 }

query-query-result-parts-edit-page-properties = Edit page properties

query-query-result-parts-page-results = Page results

query-query-result-parts-page-results-grouped = Page results grouped

query-query-result-parts-no-value = No value


query-query-result-parts-clear-grouping = Clear grouping



query-query-result-parts-a-row-with-several-tags-appears-in-every-matching-group = A row with several tags appears in every matching group, so these counts can add up to more than the result.





query-query-result-parts-this-query-has-no-conditions-so-it-shows-nothing-as = This query has no conditions, so it shows nothing, as in Logseq.

query-query-result-parts-on-its-own-it-matches-nothing = On its own it matches nothing.

query-query-live-preview-live-results = Live results


query-query-live-preview-running = Running…

query-query-live-preview-page = page






query-query-live-preview-nothing-matches-these-conditions = Nothing matches these conditions.



query-query-live-preview-more-pages-match-than-are-shown = More pages match than are shown.

query-query-live-preview-more-blocks-match-than-are-shown = More blocks match than are shown.

query-query-builder-query-text = Query text

query-query-builder-raw-query-dsl = Raw query DSL

query-query-builder-query-text-tql = Query text (TQL)

query-query-builder-query-expression = Query expression

query-query-builder-checking = Checking…

query-query-builder-save-query-text = Save query text

query-query-builder-in-a-disabled-condition-the-query-still-runs = { " " }(in a disabled condition — the query still runs)

query-query-builder-show-me = Show me

query-query-builder-properties-in-this-graph = Properties in this graph:{ " " }




# Fixed labels and operator phrases in the visual condition editor.
query-filter-condition = Condition
query-filter-page-tag-reference = Page / tag reference
query-filter-task-marker = Task marker
query-filter-priority = Priority
query-filter-property = Property
query-filter-scheduled = Scheduled
query-filter-deadline = Deadline
query-filter-in-a-journal-page = In a journal page
query-filter-between-dates = Between dates
query-filter-full-text-search = Full-text search
query-filter-page = Page
query-filter-page-property = Page property
query-filter-page-tags = Page tags
query-filter-references = references
query-filter-does-not-reference = does not reference
query-filter-is-any-of = is any of
query-filter-is-none-of = is none of
query-filter-is = is
query-filter-is-not = is not
query-filter-is-set = is set
query-filter-is-not-set = is not set
query-filter-is-a-journal-page = is a journal page
query-filter-is-not-a-journal-page = is not a journal page
query-filter-between = between
query-filter-not-between = not between
query-filter-contains = contains
query-filter-does-not-contain = does not contain
query-operator-in-namespace = is in namespace
query-filter-not-in-namespace = is not in namespace
query-filter-matches = matches
query-filter-does-not-match = does not match

# Helper-return and component-prop interface text (GH #107).
query-prose-open-tasks = Open tasks
query-prose-any-task = Any task
query-prose-a-b-or-c = A, B or C
query-prose-move-group = Move group
query-prose-select-group = Select group
query-prose-group-actions = Group actions
query-prose-group-enabled = Group enabled
query-prose-move-condition = Move condition
query-prose-select-condition = Select condition
query-prose-condition-enabled = Condition enabled
query-prose-row-actions = Row actions
query-prose-condition-operator = Condition operator
query-prose-page-property = Page property
query-prose-property = Property
query-prose-not = not
query-prose-matches = matches
query-prose-reading-this-graph-s-properties = Reading this graph's properties…
query-prose-type-to-search-page-names-and-block-text = Type to search page names and block text.
query-prose-page = Page
query-prose-block = Block
query-prose-what-this-query-selects = What this query selects
query-prose-group-the-selected-conditions = Group the selected conditions
query-prose-disabled-by-group = disabled by group
query-prose-disabled = disabled
query-prose-journal-date = Journal date
query-prose-scheduled = Scheduled
query-prose-deadline = Deadline
query-prose-any-date = Any date
query-prose-count = Count
query-prose-sum = Sum
query-prose-avg = Avg

# Complete query clauses; variables containing chip segments may be reordered.
query-prose-advanced = ⟨advanced⟩
query-prose-today = today
query-prose-yesterday = yesterday
query-prose-tomorrow = tomorrow
query-prose-journal-date-field = journal date
query-prose-not-group = NOT
query-prose-everything = everything
query-prose-nothing = nothing
query-prose-on-journal = on journal page
query-prose-any-status = Any status
query-prose-no-markers = no markers
query-prose-no-levels = no levels
query-prose-field-value = { $field }: { $value }
query-prose-on-page = page: { $value }
query-prose-in-namespace = namespace: { $value }
query-prose-page-property-clause = page { $inner }
query-prose-page-tags = page tags: { $value }
query-prose-property-any = { $field }: any
query-prose-task = task: { $value }
query-prose-priority = priority: { $value }
query-prose-content = text: "{ $value }"
query-prose-search = search: { $value }
query-prose-date-range = { $low } to { $high }
query-prose-off = { $inner } (off)
query-prose-negation = not { $inner }
query-prose-parentheses = ({ $inner })
query-prose-sentence = { $subject } where { $predicate }
query-prose-attr-content = text
query-prose-attr-task = task
query-prose-attr-priority = priority
query-prose-attr-scheduled = scheduled
query-prose-attr-deadline = deadline
query-prose-attr-created-at = created
query-prose-attr-last-modified-at = last modified
query-prose-attr-name = name
query-prose-attr-journal = journal
query-prose-attr-day = date
query-prose-attr-namespace = namespace
query-prose-attr-used-as-tag = used as a tag
query-prose-attr-key = key
query-prose-attr-value = value
query-prose-attr-atom-count = values
query-prose-property-operator =
    { $op ->
       *[is] { $cardinality ->
                [list] contains this value
               *[scalar] is
            }
        [is_not] { $cardinality ->
                [list] does not contain this value
               *[scalar] is not
            }
        [gt] is more than
        [ge] is at least
        [lt] is less than
        [le] is at most
        [between] is between
        [before] before
        [on_or_before] on or before
        [after] after
        [on_or_after] on or after
        [contains] contains
        [does_not_contain] does not contain
        [starts_with] starts with
        [ends_with] ends with
        [references] references
        [does_not_reference] does not reference
        [is_checked] is checked
        [is_unchecked] is unchecked
        [is_set] is set
        [is_not_set] is not set
        [is_blank] is blank
        [has_other_value] has a value other than
    }
query-prose-property-test =
    { $op ->
       *[is] { $field } is { $value }
        [is_not] { $field } is not { $value }
        [gt] { $field } is more than { $value }
        [ge] { $field } is at least { $value }
        [lt] { $field } is less than { $value }
        [le] { $field } is at most { $value }
        [between] { $field } is between { $value }
        [before] { $field } before { $value }
        [on_or_before] { $field } on or before { $value }
        [after] { $field } after { $value }
        [on_or_after] { $field } on or after { $value }
        [contains] { $field } contains { $value }
        [does_not_contain] { $field } does not contain { $value }
        [starts_with] { $field } starts with { $value }
        [ends_with] { $field } ends with { $value }
        [references] { $field } references { $value }
        [does_not_reference] { $field } does not reference { $value }
        [is_checked] { $field } is checked
        [is_unchecked] { $field } is unchecked
        [is_set] { $field } is set
        [is_not_set] { $field } is not set
        [is_blank] { $field } is blank
        [has_other_value] { $field } has a value other than { $value }
    }
query-prose-attribute-test =
    { $op ->
       *[eq] { $field } is { $value }
        [not_eq] { $field } is not { $value }
        [lt] { $field } < { $value }
        [le] { $field } ≤ { $value }
        [gt] { $field } > { $value }
        [ge] { $field } ≥ { $value }
        [between] { $field } between { $value }
        [in] { $field } is one of { $value }
        [not_in] { $field } is none of { $value }
        [like] { $field } contains { $value }
        [starts_with] { $field } starts with { $value }
        [match] { $field } matches { $value }
        [regex] { $field } matches regex { $value }
        [is_set] { $field } is set
        [is_not_set] { $field } is not set
        [is_blank] { $field } is blank
    }
query-prose-boolean =
    { $value ->
        [true] yes
       *[false] no
    }
query-prose-group =
    { $kind ->
       *[and] AND
        [or] OR
    }
query-prose-subject =
    { $family ->
        [both] Pages and blocks
        [page] Pages
       *[block] Blocks
    }
query-prose-all =
    { $family ->
        [both] All pages and blocks
        [page] All pages
       *[block] All blocks
    }
query-prose-date-bound =
    { $op ->
       *[ge] from { $when }
        [le] until { $when }
    }
query-prose-relative-days = { NUMBER($count, useGrouping: 0) } { $count ->
        [one] day
       *[other] days
    } { $direction ->
        [ago] ago
       *[ahead] ahead
    }
query-prose-relative-weeks = { NUMBER($count, useGrouping: 0) } { $count ->
        [one] week
       *[other] weeks
    } { $direction ->
        [ago] ago
       *[ahead] ahead
    }
query-prose-relative-months = { NUMBER($count, useGrouping: 0) } { $count ->
        [one] month
       *[other] months
    } { $direction ->
        [ago] ago
       *[ahead] ahead
    }
query-prose-relative-years = { NUMBER($count, useGrouping: 0) } { $count ->
        [one] year
       *[other] years
    } { $direction ->
        [ago] ago
       *[ahead] ahead
    }
query-prose-list = { $left }{ $position ->
        [middle] { ", " }
        [last-many] { $connector ->
            [and] { ", and " }
           *[or] { " or " }
        }
       *[last] { $connector ->
            [and] { " and " }
           *[or] { " or " }
        }
    }{ $right }
query-prose-relation = { $quant } { $relation }: { $inner }

query-prose-not-symbol = not { $symbol }
query-prose-invalid-regex = The regular expression is invalid: { $error }
query-prose-regex-summary = Matches page names or block text using the case-sensitive regular expression /{ $pattern }/.
query-prose-search-summary = Matches page names or block text { $group }.
query-prose-search-groups-summary = Matches page names or block text when it is { $groups }.
query-prose-search-and = { $left } and { $right }
query-prose-search-or = { $left }; or { $right }
query-prose-search-term = { $kind ->
        [exclude-exact] excluding the exact phrase “{ $value }”
        [exclude] excluding “{ $value }”
        [exact] containing the exact phrase “{ $value }”
       *[contains] containing “{ $value }”
    }
query-prose-aggregate-field = { $verb } of { $field }

query-prose-advanced-clause = { $advanced } { $clause }
query-prose-aggregate-unavailable = Unavailable ({ $reason ->
        [non_finite] non finite
        [division_by_zero] division by zero
        [empty_group] empty group
       *[non_numeric] non numeric
    })
query-prose-formula-statistics-notice = Exact statistics by formula are not supported yet. Overall statistics are shown.
