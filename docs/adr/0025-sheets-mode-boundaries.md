# 0025. Sheets mode boundaries: entry, exit, click semantics, flow-out

- **Status:** Accepted (decided autonomously during the Sheets build mandate — Martin should review)
- **Date:** 2026-07-06

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

- **Click on cell text → text edit**, caret at the click point — the same
  mousedown+preventDefault gesture and span→caret mapping as outline blocks;
  links/chips inside cells keep their `FORBID_EDIT_SELECTOR` behavior. One
  click, not click-select-then-click-edit (Tine is an outliner first; its cells
  feel like blocks, not Excel cells).
- **Click on cell whitespace** (padding / empty area below text) → **cell
  selection** (select mode), no caret.
- **Click on grid chrome** (gap lines near a ruling = seam target per §4.2;
  the grid's outer frame / its bullet row) → seam selection or whole-grid
  outline selection respectively.
- **Keyboard entry:** with the grid block selected in the outline, `Enter` or
  `→` enters **cell selection on the top-left cell** (row 1 col 1; the header
  row, if any, is still enterable — it's real content). Typing then replaces,
  `Enter` again edits — the standard ladder.
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
- Cell-selection-on-whitespace vs edit-on-text needs a reliable hit test
  (text node hit vs padding hit) — the implementation must use the same
  caret-from-point machinery as click-caret, falling back to cell selection
  when no text position resolves. This is the fiddly bit; it gets its own
  component tests in Phase 2.
- Flow-out semantics tie grid navigation into the outline's selection store —
  the grid component needs an escape hatch callback rather than owning outline
  selection itself.
