# 0034 — One spatial nav model for sheets and panes: shared key protocol, separate steppers, contract-tested

Status: accepted (2026-07-08)

## Context

After the pane-select nav semantics settled (ADR 0033), Martin asked whether
grid (sheet) navigation works the same way, whether it is one codebase, and
whether the sameness can be enforced structurally rather than by ADR prose.

The two surfaces had converged on the same *model* independently:

- targets are **regions** (cells / panes) and **boundaries** (row/col seams /
  pane seams, edge segments, window edges);
- arrows **alternate region → boundary → region** (a cell's arrow selects the
  seam first; a pane's arrow selects the edge segment or seam first);
- a perpendicular arrow on a boundary **slides along the line** to the
  neighbor's boundary (sheets had this from the start; panes gained it in the
  ADR 0033 lateral-nav amendment — which *was* the reconciliation);
- **Enter / typing on a boundary materializes a new region** there (insert
  row/column and edit; split pane as mirror/embryo), typing prefilling the
  editor/switcher;
- **Escape descends one rung**; **Delete removes**.

But they shared **zero code**: each hand-rolled its own KeyboardEvent
decoding (`arrowDir`/`printableKey` in `sheet/selection.ts`,
`directionForKey`/`isPrintableKey` in `keybindings.ts`), which is exactly the
layer where silent drift would start.

## Decision

**One key protocol, two steppers, one contract.**

- **Shared key→intent decoder** (`src/navProtocol.ts`): both
  `handleCellSelectionKey` and `handlePaneSelectKey` decode navigation keys
  through `decodeNavIntent` → `step / extend / activate / remove / overtype /
  dismiss`. Mod-chords (ctrl/meta/alt) are never nav — the decoder declines
  them and each surface keeps its own chords (mod+arrows content move, mod+d/r
  fill, Tab, Ctrl+K…). Both direction types (`PaneDirection`,
  `CellDirection`) are aliases of the protocol's `NavDirection`.
- **The steppers stay separate — deliberately.** A sheet is a uniform integer
  lattice: "up" has an exact logical answer independent of pixel layout (and
  must, with lazy rendering and variable cell heights). A pane layout is an
  arbitrary binary tiling where only geometry defines adjacency; its stepper
  needs leading-boundary clearing, cross-span overlap, rank, and the widening
  ladder — problems a lattice doesn't have. Forcing either through the other's
  substrate would be a downgrade, not a unification.
- **A dual-harness contract test** (`src/navModel.contract.test.ts`) pins the
  shared invariants by driving BOTH key handlers with the same key sequences
  in an equivalent 1×2 world: boundary-first stepping, lateral slide to the
  *neighbor's* boundary (owner-checked — orientation alone was too weak to
  catch a disabled lateral rule), materialize-on-Enter, create-and-type,
  Escape descent. Future drift on either surface fails a test naming it.

## Known, intentional divergences

- **Extend (shift+arrow)**: sheets grow a range; panes currently step
  (span-widening rungs are backlogged; stepping beats a dead key until then).
- **Remove**: sheets delete a row/column *at a boundary* (Backspace=before,
  Delete=after); panes close *a region*. Dual, not identical — a pane seam
  separates whole subtrees, so sided seam-deletion would close several panes
  at once. Candidate for later unification if Martin wants it.
- **Widening ladder** (press outward again: segment → seam → window edge) is
  pane-only — it navigates the layout *tree*; a flat lattice has no scopes.
  Sheets' analog is the Esc-ladder ascent through hosted subgrids.
- **F2** activates in sheets (spreadsheet convention), not in pane mode.

## Consequences

- The protocol table (which key means what) can no longer diverge between the
  surfaces; the model invariants are executable, not just prose.
- Behavioral changes to either surface's shared semantics must update the
  contract test — making "we're changing the shared model" an explicit,
  reviewable act.
- New tiling-like surfaces (e.g. a future whiteboard) should decode through
  `navProtocol.ts` and join the contract suite.
