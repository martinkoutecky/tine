# Sheets — build progress (branch `sheets`)

**This file is the single source of truth for the Sheets build state.** Any
session resuming this work: read this file FIRST, then the spec
([docs/breadth-grid-spec.md](../breadth-grid-spec.md)), then continue from
"Next step" below. Standing order: `tine-sheets-build-mandate` memory
(Martin, Jul 6 2026) — build autonomously on branch `sheets`, push after each
meaningful chunk, **never merge/rebase into master**, no version tag, no deploy
to `~/research/tine`. Martin is unavailable for testing.

## Phase checklist (spec §11)

- [ ] **Phase 0 — perf spike (throwaway, go/no-go)** ← IN PROGRESS
  - [ ] (D) render: 50×10 grid + nested sub-grid + live editor on WebKitGTK;
        keystroke→paint + resize-reflow latency vs jank budget (≈16 ms typical /
        50 ms worst); pick auto-fit strategy (CSS table auto-layout vs measured
        fixed tracks). *Caveat: uni box is headless/no-GPU — Xvfb numbers are a
        pessimistic proxy; record + proceed unless order-of-magnitude over.*
  - [ ] (B) query: `run_query("TODO")` cold (incl. cache build) + warm, +
        edit-while-board-visible re-scan. *Martin's real graph is unavailable
        (and off-limits as a corpus) — measure on a synthetic graph at ≥ real
        scale and record the substitution.* Budget: warm scan ~10–30 ms; if
        blown, facet indices move v2→v1.
  - [ ] Go/no-go + auto-fit + indices decisions recorded here.
- [ ] **Phase 1 — encoding + read-only positional grid** (render `tine.view:: grid`
      children as positional table; logical-matrix pass; §3.7 round-trip gate as
      an automated test EARLY; settle §13.2 mode-boundary + §13.4 header row as ADRs)
- [ ] **Phase 2 — editable cells + modality + full §4.3 keyboard** (largest phase)
- [ ] **Phase 3 — field-keyed table + query rowSource + task kanban** (showcase)
- [ ] **Phase 4 — Hierarchify/Flatten + board + aggregates**
- [ ] **Phase 5 — recursion + colors + polish**

## Decisions made (by Claude, per mandate)

_(none yet — each gets an ADR or spec edit + a row here)_

## Decisions Martin should review

_(none yet)_

## Next step

Phase 0: build the disposable render spike + synthetic-graph query benchmark;
record numbers + go/no-go above.

## Working notes

- **Branch hygiene:** the checkout had pre-existing uncommitted edits from
  another session (`docs/plans/theme-gallery.md`,
  `src-tauri/gen/schemas/acl-manifests.json`) — left untouched and uncommitted;
  never `git add -A` / `commit -a` / stash / checkout / clean on this tree.
  The BACKLOG.md Flathub hunk (Martin's Jul 6 decision) was committed together
  with the Sheets "Now" entry so it isn't lost.
- Commits on this branch: push with `git push origin sheets` after each chunk.
