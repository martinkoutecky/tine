## Changed since you last looked (git graphs; ADR 0079)
# $count (number): changed or removed blocks on this page since you last left it.
seen-header-count = { NUMBER($count, useGrouping: 0) } { $count ->
        [one] change
       *[other] changes
    } since you last looked
seen-mark-page = Mark page seen
seen-previous-change = Previous change
seen-next-change = Next change
# $index (number): which change the last step landed on, counting from 1.
# $total (number): how many changes the page has.
seen-change-position = { NUMBER($index, useGrouping: 0) } of { NUMBER($total, useGrouping: 0) }
# $index (number): which change the last step landed on, counting from 1.
# $total (number): how many changes the page has.
seen-change-position-wrapped = { NUMBER($index, useGrouping: 0) } of { NUMBER($total, useGrouping: 0) }, wrapped around
seen-removed-block = A block was removed here
seen-note-identity-unknown = No git identity is set for this graph, so only commits Tine made under its placeholder name count as yours.
seen-note-unattributed = This page has a long history since you last looked, so every change in it counts, including yours.
seen-note-unattributed-line = Some of these may be your own edits.
# $name (string): the page's name.
seen-page-marked = { $name } marked seen
seen-graph-marked = The whole graph is marked seen
seen-state-forgotten = Seen state forgotten for this graph
# $error (string): why marking failed.
seen-mark-failed = Couldn't mark as seen: { $error }
# $error (string): why forgetting failed.
seen-forget-failed = Couldn't forget the seen state: { $error }
seen-settings-label = Changed since you last looked
seen-settings-hint = On a git graph, Tine marks blocks other people or tools committed since you last left a page. This is stored on this device only, never in the graph. Uncommitted edits are invisible, and your own commits never count.
seen-settings-mark-graph = Mark whole graph seen
seen-settings-forget = Forget seen state
