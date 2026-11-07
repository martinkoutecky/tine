# 0025. Sheets mode boundaries: entry, exit, click semantics, flow-out

- **Status:** Accepted (decided autonomously during the Sheets build mandate — Martin should review)
- **Date:** 2026-07-06
- **Amended:** 2026-11-07 — D-20261105-GE / GH #874: grids enter Edit and walk reading order. Table and board keys are unchanged.

## Context

A grid has two interiors: Edit walks the cell texts and their child outlines;
Cell Selection manipulates cells, ranges and seams. D-20261105-GE makes editing
the entry mode, following the outline's muscle memory. A grid remains one
opaque block for outline-level selection. Click-point caret mapping uses lsdoc
spans; links, chips, media and other controls retain their own pointer actions.

## Decision

- **Single-click on cell text or whitespace → Edit**, at the click point when
  rendered-span mapping resolves it, otherwise at the text's end. A click in a
  nested block edits that block. Pointer release distinguishes a click from a
  drag across cells, which selects a range. Double-click in the editor selects
  a word through the native text editor. Shift-click extends the cell range.
- **Click on grid chrome** (gap lines near a ruling = seam target per §4.2;
  the grid's outer frame / its bullet row) → seam selection or whole-grid
  outline selection respectively.
- **Keyboard entry:** `Enter` or `→` on an outline-selected grid edits the
  top-left cell. Down at the grid block's last text row also edits that cell;
  Up from the block below edits the last visible block in the bottom-left cell.
  Vertical moves retain the caret's visual column, clamped by the destination.
- **Edit navigation:** arrows move in text until its visual edge. Left/Right
  then walk reading order through cell outlines and cells, wrapping between
  rows. Up/Down walk the cell outline and then the cell above/below in the same
  column. A hosted sub-grid follows the same rule on the outer grid's surface.
- **Edit structure:** Enter on cell text creates its first child and transfers
  the trailing text; Alt+Enter is an alias. Tab/Shift+Tab on cell text edit the
  next/previous cell, stopping at the grid ends. Nested blocks use outline
  structure commands and multiline paste, bounded by the cell. The cell block
  cannot move, indent, split into a sibling or merge; Backspace at its first
  child's start is also a no-op.
- **Keyboard exit:** `Esc` keeps text and enters Cell Selection on the edited
  cell. The existing selection ladder continues: range → cell, nested grid →
  enclosing cell, cell → outline-selected grid. Enter/F2/typing from Cell
  Selection retain their edit/overtype behavior. Selection-mode arrows step
  through seams and stop at boundary seams.
- **Flow-out from Edit:** Up from the top row or Left from the first text
  reaches the grid block's own text. Down from the bottom row or Right from
  the final text reaches the block after the grid. Right at a row's end edits
  the next row's first cell; Left reverses this path.
- **Undo/focus safety:** navigation and selection transitions are UI state;
  committed edits and structure changes use the existing document undo and
  guarded save paths. Esc does not revert text; Undo remains available.

## Consequences

- Edit walks reading order through one shared resolver and key policy in both
  block editors. Cell Selection retains the shared boundary-first navigation
  contract (ADR 0034); table title cells and board cards retain their sheet keys.
- A cell's children are an outline with their own selection and merge boundary.
  Its positional cell block stays fixed; rows and columns are Cell Selection's job.
