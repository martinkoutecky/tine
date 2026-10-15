# Tine interface text. Graph names, paths and inserted syntax in variables stay unchanged.
sheet-can-t-convert-to-grid-children-would-collide-with-grid-rows = Can't convert to grid: children would collide with grid rows.
sheet-can-t-convert-this-parser-does-not-round-trip-inside-table-cells = Can't convert: this parser does not round-trip \| inside table cells.
sheet-couldn-t-cut-cells-clipboard-write-failed = Couldn't cut cells: clipboard write failed.
sheet-pasted-over-existing-cells = Pasted over existing cells.
sheet-undo = Undo
sheet-pasted-outline-is-too-deep = Pasted outline is too deep
sheet-couldn-t-copy-cells-clipboard-write-failed = Couldn't copy cells: clipboard write failed.
sheet-resize-column = Resize { $label } column
sheet-drag-to-resize-double-click-to-reset = Drag to resize; double-click to reset
sheet-grid-requires-markdown = Convert to pipe table is only available for markdown grids.
sheet-grid-read-only = Can't convert a read-only grid to a pipe table.
sheet-grid-title-required = Can't convert: the grid block needs a title line above its tine.* properties.
sheet-grid-too-large = Can't convert grids larger than 30 columns by 200 rows to a pipe table.
sheet-grid-empty = Can't convert an empty grid to a pipe table.
sheet-grid-row-missing = Can't convert this grid because a row is missing.
sheet-grid-row-properties = Can't convert: row hidden properties would be lost.
sheet-grid-row-content = Can't convert: row blocks must be empty.
sheet-grid-row-empty = Can't convert: empty rows cannot be represented as pipe tables.
sheet-grid-cell-missing = Can't convert this grid because a cell is missing.
sheet-grid-cell-properties = Can't convert: cell hidden properties would be lost.
sheet-grid-cell-children = Can't convert: cells with children cannot be represented in a pipe table.
sheet-grid-cell-multiline = Can't convert: cells must be single-line.
sheet-grid-cell-pipe = Can't convert: this parser does not round-trip \| inside table cells.

# Query and supporting controls (GH #107). Variables carry authored text or syntax verbatim.
sheet-load-more = Load { NUMBER($count, useGrouping: 0) } more { $kind ->
        [cards] cards
       *[other] rows
    } ({ NUMBER($shown, useGrouping: 0) } of { NUMBER($total, useGrouping: 0) })
sheet-rename-field =
    { $hasReason ->
        [1] Rename field… ({ $reason })
       *[other] Rename field…
    }
sheet-cell-refused = Couldn't change that cell{ $hasValue ->
        [1] { " " }to “{ $value }”
       *[other] { "" }
    }: the page is read-only, the row is gone, or the value doesn't fit the column.
# Human-readable labels for the fixed tine.fields:: type tokens.
sheet-type-text = text
sheet-type-number = number
sheet-type-date = date
sheet-type-datetime = datetime
sheet-type-checkbox = checkbox
sheet-type-list = list
sheet-type-ref = ref

sheet-sheet-table-this-order-cannot-be-saved-while-a-computed-column-is = This order cannot be saved while a computed column is visible.

sheet-sheet-table-some-direct-rows-are-not-loaded-no-fields-were-renamed = Some direct rows are not loaded; no fields were renamed.

sheet-sheet-table-rename-field-formula-columns-cannot-be-renamed-here = Rename field… (formula columns cannot be renamed here)

sheet-sheet-table-rename-field-only-children-backed-tables-can-rename-fields = Rename field… (only children-backed tables can rename fields)

sheet-sheet-table-edit-formula = Edit formula…

sheet-sheet-table-remove-formula = Remove formula

sheet-sheet-table-add-formula = Add formula…



sheet-sheet-table-declare-field-text = Declare field (text)

sheet-sheet-table-type = Type →

sheet-sheet-table-remove-from-schema = Remove from schema

sheet-sheet-table-remove-column = Remove column

sheet-sheet-table-empty-table = empty table

sheet-sheet-table-add-row = Add row

sheet-sheet-table-block = Block

sheet-sheet-table-clear-table-only-sort = Clear table-only sort

sheet-sheet-table-filter-disabled = Filter disabled

sheet-sheet-table-rename-field-3 = Rename { $value1 } field

sheet-sheet-table-add-column = Add column

sheet-sheet-table-property = property




sheet-sheet-table-cell-menu = Cell menu

sheet-sheet-table-clear = Clear

sheet-sheet-board-moved-out-of-this-query-s-results = Moved out of this query's results

sheet-sheet-board-all-results = All results

sheet-sheet-board-invalid-tag-name = Invalid tag name

sheet-sheet-board-group-by = Group by

sheet-sheet-board-no-grouping = No grouping

sheet-sheet-board-empty-board = empty board

sheet-sheet-board-add-tag-column = Add tag column

sheet-sheet-board-new-tag = new tag

sheet-sheet-board-new-tag-2 = new-tag


sheet-sheet-board-card-menu = Card menu

# Helper-return and component-prop interface text (GH #107).
sheet-prose-state = State
sheet-prose-priority = Priority
sheet-prose-scheduled = Scheduled
sheet-prose-deadline = Deadline
sheet-prose-tags = Tags
sheet-prose-page = Page

sheet-prose-table-only-sort = Table-only sort: { $field }
sheet-prose-title = Title
sheet-board-empty-column = (none)
sheet-aggregate-none = None
# Display names only; stored function tokens remain unchanged.
sheet-aggregate-name = { $fn ->
    [sum] Sum
    [average] Average
    [median] Median
    [min] Min
    [max] Max
    [range] Range
    [stddev] Stddev
    [earliest] Earliest
    [latest] Latest
    [empty] Empty
    [filled] Filled
    [unique] Unique
    [checked] Checked
    [unchecked] Unchecked
   *[count] Count
    }
