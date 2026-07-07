# Sheets — Martin's nit list (Jul 7 2026) + triage

**Resume state:** batch 1 (N1 layout) SHIPPED `f7ea7ea`; batch 2 (N2–N5)
SHIPPED `9e90aad` (codex; orchestrator restored the properties.test.ts
coverage codex had deleted); both deployed to ~/research/tine-sheets, e2e
30/30. Martin's SECOND nit batch (Jul 7, screenshots) = N7–N9 below —
batch 3 (N7–N9) SHIPPED `02eb21e` (codex + two orchestrator fixes: centering
had to be main-pane-based not parent-based — parent-center drifts right by
half the indent and overflows the pane at narrow widths — and the
mount-time mismeasure needed a bounded verify loop, not just delayed
re-measures; probe passes 3/3). Known limitation: nested sub-grids have
no corner Σ, so sub-grid aggregates are currently unreachable (tolerable;
revisit on request). NEXT = master→sheets merge (Martin green-lit), then
batch 4 (N10–N14) SHIPPED `c762fcd`; master merge `60663d5` done. ROUND 2
(Jul 7 night, 4 screenshots) = N15–N22 below, batch 5 spec at
`subagent-tasks/sheets-nits-batch5-round2.md`.

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

## N7 — aggregate select menu collapses before you can pick  [FIXED — batch 3, 02eb21e]
Clicking Σ opens the native `<select>`, but picking an option is
impossible — the menu collapses immediately. Root cause: the footer row
only renders while `hovering()` (SheetGrid.tsx:317); the native select
popup is outside the grid element, so moving the pointer onto it fires
`pointerleave` → hovering=false → the footer row (and the select) UNMOUNTS
→ popup dies. Structural fix = N9's click-to-pin design (the row no longer
depends on hover); belt-and-braces: keep the row mounted while any footer
cell is editing.

## N8 — breakout expands only right (off-screen) + sheets jump left on hover  [FIXED — batch 3, 02eb21e]
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

## N9 — in-flow hover Σ row makes the whole page jump  [FIXED — batch 3, 02eb21e — Martin's corner-Σ design]
The batch-1 in-flow footer row appears on hover → container grows → all
content below shifts. Martin's proposal (adopted): no configured
aggregates → NO row on hover; instead a single small Σ affordance floats
at the sheet's bottom-right corner (absolutely positioned next to/over the
border — zero layout impact), shown on hover. CLICKING it (not hovering)
pins the aggregate row open — in-flow, one deliberate layout change — and
it stays until the Σ affordance is clicked again (per-grid session-only UI
state, not persisted). Configured aggregates keep the always-visible row
(no jump — it is always present). This also structurally fixes N7.

## N10 — descend INTO a sub-grid (caret navigation from edit mode)  [FIXED — batch 4, c762fcd]
Martin's ruling (final, Jul 7): **Enter on a selected cell ALWAYS enters
edit mode** — never descends (a cell can host anything, including MULTIPLE
grids, so "descend on Enter" would be ill-defined). The ladder strictly
alternates select ↔ edit. Descent happens from EDIT mode via CARET
navigation: caret Down from text above a sub-grid → that sub-grid in
select mode, TOP edge selected; caret Up from text below a sub-grid →
select mode, BOTTOM edge selected. (Multi-grid cells fall out naturally:
the caret position picks the grid.) From the edge seam, arrows move into
cells, Enter edits a sub-grid cell — recursion. Esc keeps the shipped
batch-2 behavior (sub-grid selection → host CELL selection; Esc is "get
me out", it may collapse rungs).

## N11 — seam selection should be cell-scoped, not column-scoped  [FIXED — batch 4, c762fcd]
TreeSheets distinguishes the edge between two CELLS from the edge between
two COLUMNS. Right from a selected cell currently selects the full-height
column seam; it should select just the edge between the two adjacent
cells (e.g. Sweet|12), which also visually preserves "another Right lands
on the 12 cell". Scope for the batch: seam RENDERING + selection become
cell-local; typing at a seam stays COLUMN-insert, but the edit starts in
the new cell AT THE SEAM'S ROW — Martin's point: the cell-local highlight
is exactly what makes the typing target visually unambiguous (a full
column-edge highlight leaves "which cell gets my typing?" unclear).
(Ragged per-row cell insertion = possible follow-up, tree geometry allows
it — ask Martin before building that.)

## N12 — click should SELECT a cell, not enter edit mode  [FIXED — batch 4, c762fcd; ADR 0025 amended]
Single click currently enters edit mode. TreeSheets-style ruling: click →
select mode (edit is one Enter away); double-click → edit directly.

## N13 — multi-cell selection via mouse  [FIXED (scoped) — batch 4, c762fcd; drag + shift-click ranges, shortcuts in help; keyboard suite awaits Martin's re-review]
Martin: "currently no multi-cell selection via either mouse or keyboard".
Code says keyboard ranges EXIST (Shift+Arrows extend, Shift+Space rows,
Ctrl+Space cols, Ctrl+A all, Ctrl+C/X copy/cut, Ctrl+D/R fill, a paste
path) — likely hidden behind N12's click-to-edit. Martin's ruling: "don't
waste too much time here — add mouse drag and fix click→select, and I'll
review it again." So batch 4 = mouse drag range selection (pointer down
on a cell + drag = range, keyboard anchor model) + N12 + surfacing the
existing bindings in the shortcuts help; deep verification of the
keyboard suite waits for his re-review.

## N14 — board card drag fights outline multi-block selection  [FIXED — batch 4, c762fcd]
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

## N15 — query-board justification still a mess  [batch 5]
Round 2 (Jul 7 night, screenshot): the §4 query kanban overflows past the
window's right edge, uncentered, no scrollbar. ROOT CAUSE (confirmed in
code): macro-rendered query sheets (Macro.tsx:481/484) are NOT wrapped in
SheetContainer — they never got the breakout/centering/scroll treatment
that children-source sheets got in batches 1/3. Fix: same wrapper for the
macro path.

## N15b — ghost empty kanban under the query board  [batch 5]
Martin: "what is the third empty kanban under the working one?" ROOT
CAUSE (confirmed): the query block renders TWICE — the macro path renders
the query board, and Block.tsx's children-Switch ALSO fires (sheet().view
=== "board") rendering a children-source SheetBoard with zero children =
empty LATER/NOW/DONE columns. Fix: gate the children-sheet Match arms on
the block body NOT being a macro (detectMacro).

## N16 — hover scrollbars obscure content incl. the Σ  [batch 5]
Tables show tiny vertical+horizontal scrollbars on hover that obscure
content; Martin couldn't click the Σ at all (WebKitGTK scrollbars are
fatter than my Chromium probe's). ROOT CAUSE: the corner Σ toggle sits at
right:-7/bottom:-7 INSIDE the overflow-x:auto container → 7px of
scrollable overflow on both axes. Fix: restructure — outer
.block-sheet-container (position:relative, no scroll) > inner
.sheet-scroll (the ONE scroll region) > face; corner overlay lives in the
outer. Zero overflow when content fits → no scrollbars at all.

## N17 — tag board: last card leaves → column vanishes; want add-column  [batch 5]
Correct but unexpected. Martin's fix (agreed): an "add column" affordance
on tag boards = add a new tag. Design: ghost "+ new tag" column at the
right end; click → inline name input → creates a session-local empty
column (a tag exists on disk only via cards, so the empty column lives in
UI state until a card is dropped/created in it — same pin-style session
map as the Σ toggle).

## N18 — click-select landed inconsistently; select+edit at once; shake  [batch 5]
In §1's grid: clicking a first-column cell still enters EDIT mode;
clicking a second-column cell yields BOTH select highlight AND a caret,
and the table "shakes" left-right. Batch 4's pointerSelection did not
become the single entry point — some legacy mousedown-edit path still
fires (double-handling). Fix: ONE pointer entry for all cell mousedowns
(pointerSelection), kill/route the legacy handlers, double-click = edit.
The shake is N19's width instability triggered by the spurious edit.

## N19 — column width must not change select↔edit  [batch 5]
The column renders wider when a cell is in edit mode than in select mode.
Martin's rule: the table NEVER moves while navigating or entering edit;
only actual content changes may reflow. Fix: freeze effective column
widths for the duration of an edit session (or make the editor fit the
cell's existing box) — no max-content re-measure from editor mounting.

## N20 — sub-grid descent selects the wrong edge  [batch 5]
Down from edit of the host cell ("inner grid in this cell", caret in
text) should select the SUB-GRID's TOP edge; Martin observes the HOST
cell's bottom edge instead ("that edge should be selectable when I am in
select mode of the outer grid" — it belongs to the outer grid). Reproduce
in the §2 demo, find why selectTopRowSeamAfterEdit lands on the wrong
grid/edge (or renders as if it did), fix + component test.

## N21 — seam ladder broken/incorrectly rendered in grids  [batch 5]
From Sweet (§1, row Apples|Sweet|12|30Kč): ArrowLeft selects the WHOLE
Product|Taste column edge (should be only the Apples|Sweet edge segment
in Sweet's row — the cell-scoped rendering shipped in batch 4 is not
working in the grid); another Left lights NOTHING (dead state); another
Left selects the grid's leftmost whole edge. Expected ladder
(alternation): Sweet → seam(Apples|Sweet, that row) → cell Apples →
seam(grid left edge, that row) → stop. Reproduce, fix model + rendering,
selection tests + e2e.

## N22 — the two "+" chips need a real design  [batch 5 — proposal accepted-pending-Martin]
Tooltips explain them but "better design is called for; think about it."
PROPOSAL (implementing unless Martin objects): Notion-style ghosts —
"add column" = a ghost header cell at the far right of the header row
(faint +, label on hover, only visible while hovering the sheet); "add
row" = a ghost row spanning the bottom (faint +). Both zero-layout-shift
(they occupy the existing tail space), discoverable in place, no
mystery chips in the corner.

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
