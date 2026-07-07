# Sheets — Martin's nit list (Jul 7 2026) + triage

Captured verbatim-in-spirit from his post-Phase-7 testing; root causes
investigated before triage. Batch polish pass runs against this list.
Status legend: FIXED / IN PROGRESS / PROPOSED (needs Martin's call).

## N1 — cramped layout (his biggest)  [IN PROGRESS — batch 1]
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

## N2 — kanban drag has no ghost; cursor becomes a caret  [batch 2]
The dragged card only shades in place; nothing follows the pointer, and
the text-caret cursor confuses. Fix: floating drag ghost (fixed-position
clone, pointer-events:none), `cursor: grabbing` during drag,
selection/caret suppressed.

## N3 — Esc from a nested sub-grid should walk UP the ladder  [batch 2]
Esc inside the inner grid should land on the OUTER cell containing the
sub-grid (Esc walks up, Enter walks down — ADR 0025); today it exits
toward the outline.

## N4 — Enter on a sub-grid cell reveals `tine.view:: grid`  [batch 2]
The cell editor shows the block's config property lines. In a sheet cell,
the editor should present only the visible body and splice the hidden +
`tine.*` lines back on commit (same split/join the overtype path already
uses).

## N5 — kanban cards disappear when moved to NOW  [ROOT-CAUSED; guard in batch 2]
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
