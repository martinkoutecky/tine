# Tine interface text. Graph names, paths and inserted syntax in variables stay unchanged.
# Number of blocks referencing the current block.
editor-references-linked-reference = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } Linked Reference
   *[other] { NUMBER($count, useGrouping: 0) } Linked References
    }
editor-references-reference-page-groups = Reference page groups
editor-references-collapse-all = Collapse all
editor-references-expand-all = Expand all
editor-references-references-from = { $action ->
    [expand] Expand references from { $page }
   *[collapse] Collapse references from { $page }
    }
