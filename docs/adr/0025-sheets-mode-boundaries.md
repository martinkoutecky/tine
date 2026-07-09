# 0025. Sheets mode boundaries: entry, exit, click semantics, flow-out

- **Status:** Accepted (decided autonomously during the Sheets build mandate — Martin should review)
- **Date:** 2026-07-06
- **Amended:** 2026-07-07 — batch-4 review overruled click-on-text edit entry.

## Context

Spec §13.2: where the caret/selection lands entering and exiting a grid, click
*into* a cell vs *onto* the grid, and flow-out at borders — "the part most
likely to feel wrong if rushed." The frame (§4): a grid is a block whose
edit-interior is 2-D; the outline's select→edit ladder gains one 2-D rung
(`outline selection → cell selection → text edit`); a grid is opaque to
outline-level selection; flow-out at top/bottom, never wrap. Tine's existing
conventions that bear on this: edit entry is MOUSEDOWN + preventDefault with
click-point caret mapping via lsdoc spans (`tine-click-caret-inline-spans`),
and `FORBID_EDIT_SELECTOR` excludes links/chips/media from edit entry.

## Decision

- **Single-click on any cell text or whitespace → cell selection** (select
  mode), no caret. This overrules the original click-on-text-edits boundary:
  the spreadsheet/TreeSheets parity is more important in use, and click-select
  makes mouse range selection reachable without first falling into edit mode.
- **Double-click on a cell → text edit**, caret at the click point when the
  rendered-span caret mapping resolves one; otherwise edit starts at the normal
  cell edit position. `Enter`, `F2`, and typing from cell selection keep their
  shipped edit/overtype behavior.
- **Click on grid chrome** (gap lines near a ruling = seam target per §4.2;
  the grid's outer frame / its bullet row) → seam selection or whole-grid
  outline selection respectively.
- **Keyboard entry:** with the grid block selected in the outline, `Enter` or
  `→` enters **cell selection on the top-left cell** (row 1 col 1; the header
  row, if any, is still enterable — it's real content). Typing then replaces,
  `Enter` again edits — the standard ladder.
- **Sub-grid descent:** `Enter` on a selected cell always enters edit mode; it
  never descends. From the sheet-cell editor, `ArrowDown` at the last text row of
  a cell that itself renders a child grid commits the edit and selects the
  child grid's top edge seam. The current rendering always places the cell's
  text above its children, so "ArrowUp from text below the sub-grid" has no UI
  position yet.
- **Re-entry memory:** while a page stays mounted, each grid remembers its last
  selected cell (in-memory signal, never persisted) and re-entering lands
  there; fresh mounts land top-left.
- **Keyboard exit:** `Esc` from text edit → cell selection; `Esc` from cell
  selection → outline selection of the whole grid block. `←` past the leftmost
  column also exits to outline selection (the §4 ladder's ascend).
- **Flow-out:** `↓` on the bottom row moves outline selection to the block
  *after* the grid; `↑` on the top row to the block *before* it. No wrap. `→`
  past the rightmost column does nothing (right has no outline meaning).
- **Undo/focus safety:** mode transitions never mutate the document; they are
  pure UI state (per-grid signals), so none of this enters undo history.

## Consequences

- Matches the outline's muscle memory (mousedown edit entry, Esc ladder) while
  adding the one 2-D rung the spec calls for; no new global modality.
- Double-click edit still uses the same caret-from-point machinery as outline
  click-caret, but single-click no longer depends on a text-vs-padding hit test.
- Flow-out semantics tie grid navigation into the outline's selection store —
  the grid component needs an escape hatch callback rather than owning outline
  selection itself.
