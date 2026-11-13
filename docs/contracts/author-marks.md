# Author marks (GH #892, #893, #896)

`tine.author` names the author of one block's words. Absence means the graph
owner; there is no inheritance. Editing and moving keep that property. This
display contract adds no graph writes or file format.

Ordinary blocks written by someone else draw a two-pixel gutter line centred
in the existing bullet-to-text gap (six pixels for dots, four for numbers).
Showing or hiding it never changes text
coordinates. It remains visible in both block editors. Comments and replies
keep their existing author headers and have no author gutter line.
The bulletless host of a whole-block embed also has no mark of its own; the
source block draws its own attribution.

Consecutive siblings in one rendered list with the same author share a
continuous line and a chip on the first block. An owner block, author change,
different parent/page, embed, comment thread or child outline breaks the run.
A block with children may finish a run but does not connect to its next sibling,
including when folded; a child starts its own sibling context. Embed hosts are
singletons (including a block containing an inline embed; code examples remain
literal). Each rendered subset starts independently, including embedded and
sidebar copies. Hovering or focusing an individual gutter mark shows that
block's author. The first chip retains its **Remove author** menu.

**Remove author** is a palette command and bindable shortcut, without a default
key. It acts on the complete block selection (folded descendants included), or
the edited block when there is no selection. The block context menu acts on
that same selection when its pointer is inside it, otherwise on the pointer
block. It is offered when at least one target is writable and carries
`tine.author`; read-only blocks and blocks without attribution are skipped.
The shared action removes only `tine.author`, retaining other text/properties,
through the property writer in one undo step and one guarded save per changed
page. Availability stops at the first matching target; execution costs
O(selected property bytes), plus the existing affected-page undo/save cost.

**Show/Hide author marks** is an independent palette command, rebindable under
Settings → Keyboard shortcuts, without a default key. Settings → Appearance
groups its switch with **Show/Hide comments**. It controls gutter lines only
(GH #896's definition of a mark); author chips, comment headers and comment
visibility remain available. Marks are shown by default, preserving the prior
default. The device preference uses the existing display-preference writer;
refused storage leaves its value unchanged and uses the existing error toast.

Cost: one O(sibling IDs) adjacency map per rendered list, rebuilt only when
list membership/order changes. Reused ID arrays cost O(1) to admit; if a view
filter emits a new array, an O(sibling IDs) equality check compares its existing
IDs only, without reading authors or graph nodes. Equal membership keeps the
map and run memos unchanged. This same stable list feeds the renderer, so equal
IDs do not rerun whole-list geometry or run derivation. A row compares only its two rendered neighbours,
using cached parser facets and parent/page/child metadata. An ordinary text edit
invalidates at most that row and its two neighbours for authorship, never a
whole-page authorship computation or graph scan. No disk read, IPC, save,
fsync, lock, index or transport record is added. The preference writes one
localStorage opt-out (`1`); graph bytes per edit remain unchanged.
Embed detection's opening-token gate is O(local raw bytes); its body AST
classification is cached, and it never expands or reads an embedded graph.
