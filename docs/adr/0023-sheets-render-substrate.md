# 0023. Sheets render substrate: CSS Grid with max-content tracks

- **Status:** Accepted (decided autonomously during the Sheets build mandate — Martin should review)
- **Date:** 2026-07-06

## Context

The Sheets engine (docs/breadth-grid-spec.md) needs an auto-fit layout substrate
with the TreeSheets feel: cells resize to content fluidly per keystroke. Spec §8
mandated a Phase-0 spike to choose between native `<table table-layout:auto>`
and CSS Grid, against a jank budget of ≈16 ms typical / 50 ms worst, because
DOM + WebKitGTK might not deliver what TreeSheets does with an immediate-mode
C++ canvas.

The spike (`scripts/spike-sheets-perf.mjs`) drives the real Tine binary
(WebKitGTK) under Xvfb with software rendering — a pessimistic proxy, no GPU —
injects a 50×10 editable grid with a nested 5×5 sub-grid, and measures
synchronous reflow cost (mutate → forced layout flush; double-rAF saturates at
the ~32 ms two-frame floor and cannot distinguish variants). Results
(median/p95 ms, Jul 6 2026):

| variant | keystroke | track-resize | row insert | initial layout |
|---|---|---|---|---|
| table-auto 50×10 | 4 / 8 | 5 / 9 | 1 / 2 | 8 |
| grid max-content 50×10 | 1 / 3 | 2 / 3 | 2 / 4 | 7 |
| grid fixed 50×10 | 1 / 3 | 2 / 2 | 1 / 3 | 6 |
| table-auto 200×10 | **15 / 16** | **18 / 20** | 2 / 4 | 29 |
| grid max-content 200×10 | 3 / 10 | 4 / 9 | 3 / 9 | 27 |
| grid fixed 200×10 | 2 / 7 | 2 / 5 | 3 / 9 | 26 |

Caveat: spike cells were plain contenteditable divs, not Tine block
projections — cell *build* cost is heavier in real life, but layout mechanics
(the go/no-go question) is what was measured, in a paint-pessimistic
environment.

## Decision

Phase 0 is a **GO**. We will lay out sheet faces with **CSS Grid using
`max-content` tracks** for content-sized columns (`trackModel: content`), with
per-column fixed tracks (`px` / `fr`) when the user drags a width
(`trackModel: manual`, persisted via `tine.col-widths::`). We will NOT use
native `<table>` auto-layout for the editable grid. Facing the §8 cap/log rule:
no hard cap in v1; log a console warning around ~2000 cells where the
whole-grid relayout medians approach 25–30 ms even on grid.

## Consequences

- The 16 ms budget holds with 4–16× headroom at the spec's 50×10 target, on
  software rendering; GPU machines will only be better.
- CSS Grid is also what §2 wants for `fr`-track manual sizing, span placement
  (v2 `rowspan`/`colspan` via grid lines), and the seam drag — one substrate
  for all faces that use tracks.
- Table-auto's whole-table reflow grows ~linearly with cell count (budget edge
  at 2000 cells), so grid buys the stress margin; the cost is that header/cell
  alignment is ours to manage (no `<thead>` semantics for free) and
  accessibility roles must be added by hand (`role="grid"`/`row`/`gridcell`).
- The spike script stays in `scripts/` for re-measurement on real hardware
  (run with a display: `DISPLAY=… node scripts/spike-sheets-perf.mjs`).
