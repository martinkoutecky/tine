# 0033 — Pane-select navigation semantics: overlap stepping, pane-edge segments, focus-follows-selection

Status: accepted (2026-07-08)

## Context

Martin's first field session with split view (Jul 8) surfaced four defects in
the pane-select mode shipped in S3 (ADR 0032):

1. **Stepping was center-distance-only.** ArrowDown from a tall right pane
   selected the seam *between the two left panes* (its center is a few pixels
   "below"), reading as "nothing happened". ArrowUp jumped diagonally into the
   top-left pane.
2. **Only whole-window edges existed.** With a left/right split there was no
   way to split *just the left half* horizontally — the top edge always split
   the root ("split the whole screen, then re-split the bottom half" was the
   only route to that layout).
3. **Ctrl+K acted on the focused pane, not the selected one** — the pane you
   *see* highlighted is not the one that received the page.
4. **Esc from block-select landed on an invisible dead rung** (selection
   cleared, nothing else) before a further Esc entered pane-select.

## Decision

- **Overlap-constrained stepping.** A directional step only considers targets
  whose cross-axis interval overlaps the current target's (degenerate
  intervals count via closed containment). No more diagonal or
  behind-the-other-column jumps.
- **Pane-edge segments** are a new target kind `{kind: "pane-edge", paneId,
  side}`: a pane's boundary side lying on the window edge. Enter/typing on a
  segment splits ONLY that pane (new pane on that side). Pressing *outward
  again* on a segment widens it to the whole-window edge (root split) —
  TreeSheets' "edge of the left half, then the full edge". The ghost rung is
  skipped when the segment already spans the full edge. Rank order on exact
  ties: seam > pane-edge > pane > edge.
- **Focus follows pane selection.** Arrowing onto a pane target focuses it, so
  Ctrl+K (and every focused-pane command) acts on the pane that is visibly
  selected. Ctrl+K on a pane target also exits the mode (the switcher takes
  over); Ctrl+K on a seam/edge/segment is the same gesture as typing — an
  embryo split with an empty switcher.
- **Esc from block-select climbs straight to pane-select** (2 rungs:
  edit → block-select → pane-select). Focus mode still peels first. The old
  "selection cleared, nothing active" state between them was an invisible
  dead rung that made the ladder feel broken.
- **Delete/Backspace on a selected pane closes it**, staying in the mode on
  the survivor.

## Consequences

- `computePaneGeometry` also emits `paneEdges`; the highlight for a segment is
  rendered *inside* the owning pane (`PaneEdgeSegHighlight`), so it spans
  exactly that pane's side — the affordance shows what will split.
- Whole-window edges are only reachable by pressing outward *through* a
  segment, except where they coincide (solo pane) — where both mean the same
  split anyway.
- Splitting an *intermediate* subtree's edge (deep trees: wider-than-one-pane
  but narrower-than-root) is not reachable; revisit if it ever comes up.
- The hint pill documents the per-target actions and is deliberately larger
  (2–3 lines, Martin's call: "when splitting you don't need the content under
  it").
