# Sheets — Martin's nit list (Jul 7 2026) + triage

**Resume state:** batch 1 (N1 layout) SHIPPED `f7ea7ea`; batch 2 (N2–N5)
SHIPPED `9e90aad` (codex; orchestrator restored the properties.test.ts
coverage codex had deleted); both deployed to ~/research/tine-sheets, e2e
30/30. Martin's SECOND nit batch (Jul 7, screenshots) = N7–N9 below —
batch 3 spec at `subagent-tasks/sheets-nits-batch3-footer-layout.md`.

Captured verbatim-in-spirit from his post-Phase-7 testing; root causes
investigated before triage. Batch polish pass runs against this list.
Status legend: FIXED / IN PROGRESS / PROPOSED (needs Martin's call).

## N1 — cramped layout (his biggest)  [FIXED — batch 1, Jul 7]
Shipped: measured full-width breakout (ResizeObserver + CSS vars, 20px
gutters; nested sheets in a `.sheet-cell` never break out); `.sheet-grid`/
`.sheet-table`/`.sheet-board` max-width squeeze removed — the outer
`.block-sheet-container` is the ONE scroll region; hover Σ footer is now
an IN-FLOW row below the last row (overlay CSS deleted); sticky-left first
column on grids + table title column (opaque, z-layered; sticky-TOP header
punted — WebKitGTK page-scroller fiddliness, revisit on demand); context
menu gains "Open as full page" (existing `zoomInto`, zoom path verified to
render sheet faces). Verified: table sticky probe (scrollLeft=400, title
pinned, breakout not under sidebar), wide-grid screenshot, e2e 30/30.
Symptoms: (a) default main column keeps sheets narrow — they should span
sidebar-to-sidebar by default; (b) even in wide mode a grid can stay
narrow with hidden, horizontally-scrollable text; (c) hovering a grid
overlays the Σ aggregate row on TOP of the last row, hiding what you edit.
Root causes found: `.sheet-grid { max-width: 100% }` squeezes wide tables
into the column and the nowrap cells clip behind an inner scrollbar;
`.sheet-footer-overlay { position:absolute; bottom:0 }` covers the last
row; the demo also pinned col 0 at 140px (`tine.col-widths:: 0=140`) which
truncated the long ragged-row cell (sample fixed — pin moved to Qty).
Fix (batch 1): full-width breakout container + natural width + in-flow
hover footer + sticky header/first column when scrolling. The bigger
"canvas" question → PROPOSED section below.

## N2 — kanban drag has no ghost; cursor becomes a caret  [FIXED — batch 2, 9e90aad]
The dragged card only shades in place; nothing follows the pointer, and
the text-caret cursor confuses. Fix: floating drag ghost (fixed-position
clone, pointer-events:none), `cursor: grabbing` during drag,
selection/caret suppressed.

## N3 — Esc from a nested sub-grid should walk UP the ladder  [FIXED — batch 2, 9e90aad]
Esc inside the inner grid should land on the OUTER cell containing the
sub-grid (Esc walks up, Enter walks down — ADR 0025); today it exits
toward the outline.

## N4 — Enter on a sub-grid cell reveals `tine.view:: grid`  [FIXED — batch 2, 9e90aad; Esc now CANCELS a cell edit (spreadsheet parity), commit paths splice hidden lines back]
The cell editor shows the block's config property lines. In a sheet cell,
the editor should present only the visible body and splice the hidden +
`tine.*` lines back on commit (same split/join the overtype path already
uses).

## N5 — kanban cards disappear when moved to NOW  [FIXED — samples + batch-2 toast guard, 9e90aad]
Not a store bug: the demo board's query was `(todo TODO DOING DONE)` while
the workflow is LATER/NOW — the board offers workflow columns (LATER, NOW,
DONE), so moving a card into NOW writes a marker the query doesn't match
and the card correctly leaves the results. Sample query FIXED to include
all markers. Product guard (batch 2): after a query-board move, if the
moved card is no longer in the refreshed results, toast "moved out of this
query's results" — honest, covers any filtered board.

## N6 — §9 table shows weird trailing rows  [FIXED — samples]
The explanatory note bullets were CHILDREN of the table block, so they
rendered as rows (no points → ⚠ error formula cells). Children = rows is
the contract; the notes moved out to sibling blocks (also §6's try-it row).

## N7 — aggregate select menu collapses before you can pick  [batch 3]
Clicking Σ opens the native `<select>`, but picking an option is
impossible — the menu collapses immediately. Root cause: the footer row
only renders while `hovering()` (SheetGrid.tsx:317); the native select
popup is outside the grid element, so moving the pointer onto it fires
`pointerleave` → hovering=false → the footer row (and the select) UNMOUNTS
→ popup dies. Structural fix = N9's click-to-pin design (the row no longer
depends on hover); belt-and-braces: keep the row mounted while any footer
cell is editing.

## N8 — breakout expands only right (off-screen) + sheets jump left on hover  [batch 3]
The wide kanban/table extends past the RIGHT window edge with no left
shift; hovering §6/§9 makes them jump left to the correct breakout
position. So the initial mount-time measure() computes a wrong/zero
`--sheet-breakout-shift`, and the hover-mounted footer row triggers the
ResizeObserver → re-measure → correct shift → visible jump. Also batch 1
made breakout width ALWAYS ≥ the full sidebar-to-sidebar span
(`max(normalWidth, span)`), so a modestly-wide sheet (§9) gets a
full-span container with dead space. Martin's ruling: (a) breakout sheets
are CENTERED between the sidebars in the default view, (b) layout must
NEVER change on hover. Fix: width = min(natural, span), shift computed to
center; find + fix the deterministic reason the mount-time measure is
wrong (don't paper over); N9 removes hover-driven geometry entirely.

## N9 — in-flow hover Σ row makes the whole page jump  [batch 3 — Martin's design, agreed]
The batch-1 in-flow footer row appears on hover → container grows → all
content below shifts. Martin's proposal (adopted): no configured
aggregates → NO row on hover; instead a single small Σ affordance floats
at the sheet's bottom-right corner (absolutely positioned next to/over the
border — zero layout impact), shown on hover. CLICKING it (not hovering)
pins the aggregate row open — in-flow, one deliberate layout change — and
it stays until the Σ affordance is clicked again (per-grid session-only UI
state, not persisted). Configured aggregates keep the always-visible row
(no jump — it is always present). This also structurally fixes N7.

## N10 — descend INTO a sub-grid (Down from edit mode)  [batch 4]
Martin (3rd batch): with the caret in edit mode inside a cell hosting a
sub-grid, pressing Down should put the SUB-GRID in select mode — his
ruling: "the sensible default is top edge selected". Context: batch 2
shipped Esc-up only; Enter/descend was explicitly punted as a design
question — now ruled. Rule set for the batch: Down from edit-in-host-cell
descends (top edge of the sub-grid selected); Esc from the sub-grid walks
back up (already shipped). Open sub-question (pick a default, flag it):
Enter from SELECT mode on a sub-grid host cell — descend (ladder-
consistent) vs edit the host's own text; default = descend, host text
still editable via F2/typing-overtype.

## N11 — seam selection should be cell-scoped, not column-scoped  [batch 4]
TreeSheets distinguishes the edge between two CELLS from the edge between
two COLUMNS. Right from a selected cell currently selects the full-height
column seam; it should select just the edge between the two adjacent
cells (e.g. Sweet|12), which also visually preserves "another Right lands
on the 12 cell". Scope for the batch: seam RENDERING + selection become
cell-local; typing/insert semantics at a seam stay column-insert for now
(ragged per-row cell insertion = possible follow-up, tree geometry allows
it — ask Martin before building that).

## N12 — click should SELECT a cell, not enter edit mode  [batch 4]
Single click currently enters edit mode. TreeSheets-style ruling: click →
select mode (edit is one Enter away); double-click → edit directly.

## N13 — multi-cell selection via mouse + full edit suite  [batch 4 — partly exists, VERIFY]
Martin: "currently no multi-cell selection via either mouse or keyboard".
Code says keyboard ranges EXIST (Shift+Arrows extend, Shift+Space rows,
Ctrl+Space cols, Ctrl+A all, Ctrl+C/X copy/cut, Ctrl+D/R fill, a paste
path) — so either they're broken in the real app or undiscoverable (or
N12's click-to-edit hides them). Batch 4: (a) verify each in the real
app and fix what's broken; (b) ADD mouse drag range selection (pointer
down on a cell + drag = range, consistent with the keyboard anchor
model); (c) ensure paste works into cells/ranges; (d) surface the
bindings in the help/shortcuts panel + FEATURES.md.

## N14 — board card drag fights outline multi-block selection  [batch 4]
Dragging a kanban card also drives the outline's multi-block (blue)
selection — mousedown+move on a card must not start outline block
selection. Suppress outline selection while a card drag is in progress
(and for the mousedown that initiates it).

## PROCESS — master accumulating while sheets runs  [agreed with Martin Jul 7]
Discipline: periodically merge MASTER → SHEETS (never the reverse — the
sheets→master merge stays Martin-gated). Cadence: at batch boundaries,
or whenever master lands editor/caret/mousedown work that touches shared
code. Full gates + sheets e2e after every such merge. First merge: after
batch 3 lands.

## PROPOSED — the "canvas" question (Martin asked for thinking, not just fixes)
Recommendation, three layers (details in the reply that accompanied this
capture): 1) default full-width breakout between the sidebars; 2) a sheet
that still doesn't fit becomes its own two-axis scroll region with sticky
header row + sticky first column (the spreadsheet affordance); 3) the true
canvas = ZOOM the sheet block (block zoom already exists) — add an
"expand" affordance on the sheet header/context menu so one click gives
the sheet the whole viewport. Explicitly not proposed: an
infinite-pan canvas widget (that's the v3 canvas-face territory; new
interaction model, poor fit with outline scrolling).
