# 0037. Sheet paste: mode decides nest vs splat

- **Status:** Accepted
- **Date:** 2026-07-09
- **Amends:** [0031](0031-recursive-cell-form.md) (the paste-target rule; the
  two-form model and the fingerprint handshake from 0031 still stand)

## Context

ADR [0031](0031-recursive-cell-form.md) made *every* structural grid paste onto a
selected sheet cell append a **hosted child grid** inside that cell. In practice a
grid copied and pasted back onto the surface became a grid-nested-in-a-cell — a
double-nested grid the user did not ask for. When the clipboard was a plain block
copy (foreign markdown) instead, the same cell paste dropped the text in as nested
outline bullets, which then read as a one-column sub-grid. Both were the wrong
default: the common intent when you copy a rectangle of cells and paste it
elsewhere on the grid is to lay those cells down *in the surrounding grid*
(Excel/TreeSheets "splat"), not to bury them one level deeper.

TreeSheets (studied from source) decides nest-vs-splat by a data heuristic — is
the target cell empty? — but in an outline backing store "empty block" is common
(spacer bullets), so an emptiness heuristic makes accidental destructive splats
onto blank lines a real hazard. We need a signal the user already controls and
that carries no spacer-bullet risk.

The two paste entry points already sit on different DOM targets: a paste while a
cell's `<textarea>` is focused is an *edit-mode* paste (it reaches the editor's own
`onPaste`); a paste while cells are merely *selected* reaches the global sheet
paste handler, which bails on editable targets. That existing split is the signal.

## Decision

We will let **paste mode** decide, for our own structural grid clipboard only:

- **Edit mode** (caret inside a cell) → **NEST**: the copied region becomes a
  hosted `tine.view:: grid` subgrid at the caret. (Unchanged from 0031; this is how
  you deliberately nest — enter the cell first, then paste.)
- **Select mode** (cells selected, not editing) → **SPLAT**: lay the copied M×N
  region into the surrounding grid, anchored at the selection's top-left.

Splat semantics (one undo unit, `sheet:paste-splat`):
1. **Anchor** = the selection's top-left cell; a single selected cell is its own
   anchor. The selection's *size* only provides the anchor — there is **no
   tiling** to fill a larger selection; the footprint is the clipboard's size.
2. **Grow to fit**: append rows under the grid when the footprint runs past the
   last row; when a target row is shorter than `anchor.col + width`, pad it with
   empty cells up to the anchor column, then place. Growth is only at the
   bottom/row-end; the anchor never moves.
3. **Overwrite destructively**: each footprint cell's text *and subtree* are
   replaced by the source cell's; the target cell's own hidden `id::` is preserved
   so no `((ref))` is orphaned. If any overwritten cell was non-empty, a
   non-sticky **undo toast** ("Pasted over existing cells.") is shown. The same
   toast fires on the TSV-matrix paste path.
4. A **single copied cell** is not a 1×1 splat — it falls through to plain-text
   paste (matching the existing single-cell rule).

Scope: mode selects nest-vs-splat **only for our own structural grid clipboard**
(the fingerprint-matched copy). Foreign clipboard text keeps its conventional
behavior — inline text into the cell in edit mode, TSV-matrix fill in select mode.
There is no drag gesture, so "cut then paste elsewhere" composes a move as two
atomic undo units (cut clears the source, paste splats); we deliberately do **not**
fuse them into one undo.

## Consequences

Copying a region and pasting it back on the grid now does the obvious thing, and
the accidental double-nest is gone. The emptiness heuristic is retired entirely, so
there is no spacer-bullet splat hazard; the nest gesture is explicit (enter the
cell). Splat overwrite is destructive but always one undo and toasted when it
clobbers real content.

The costs we accept: splat is a genuinely destructive region overwrite (like every
spreadsheet), mitigated only by undo + toast; and a future "Paste special →" menu
(nest-a-region / splat-a-single / insert-and-shift) and a fused single-undo
drag-move remain unbuilt (tracked in `docs/BACKLOG.md`). The renamed helper is
`splatStructuralSheetSelection` (was `pasteStructuralSheetSelection`); the
edit-mode nest helper `structuralSheetPasteNode` is unchanged.
