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
  - [x] (D) render — **GO, with 4–16× headroom.** `scripts/spike-sheets-perf.mjs`
        drives the real WebKitGTK binary under Xvfb (software rendering,
        `WEBKIT_DISABLE_COMPOSITING_MODE=1` — pessimistic), injects a 50×10
        editable grid + nested 5×5 sub-grid, measures **synchronous reflow cost**
        (mutate → forced layout flush; double-rAF saturates at the 2-frame
        ~32 ms floor and is useless as a metric). Numbers (median/p95 ms),
        Jul 6 2026, uni box:
        | variant | keystroke | track-resize (grow) | row insert | initial layout |
        |---|---|---|---|---|
        | control (1 cell) | 1 / 1 | 0 / 1 | — | 1 |
        | table-auto 50×10 | 4 / 8 | 5 / 9 | 1 / 2 | 8 |
        | grid max-content 50×10 | 1 / 3 | 2 / 3 | 2 / 4 | 7 |
        | grid fixed 50×10 | 1 / 3 | 2 / 2 | 1 / 3 | 6 |
        | table-auto 200×10 | **15 / 16** | **18 / 20** | 2 / 4 | 29 |
        | grid max-content 200×10 | 3 / 10 | 4 / 9 | 3 / 9 | 27 |
        | grid fixed 200×10 | 2 / 7 | 2 / 5 | 3 / 9 | 26 |
        Everything at the spec's 50×10 target is far inside the 16 ms budget.
        **Auto-fit strategy DECIDED: CSS Grid with `max-content` tracks**
        (`<table table-layout:auto>` rejected — whole-table reflow, ~linear in
        cell count, hits the budget edge at 2000 cells; grid max-content stays
        3–4 ms there and is also the substrate §2 wants for `fr`/manual tracks).
        Cap/log threshold: grid still fine at 2000 cells → soft log-warning
        around ~2000 cells, no hard cap needed for v1. Caveats: cells were plain
        contenteditable divs, not Tine block projections (heavier to *build*,
        but layout mechanics was the question); paint cost not isolated — but
        the environment is software-rendered, i.e. pessimistic on paint.
  - [ ] (B) query: `run_query("TODO")` cold (incl. cache build) + warm, +
        edit-while-board-visible re-scan. *Martin's real graph is unavailable
        (and off-limits as a corpus) — measuring on a synthetic graph at ≥ real
        scale (10k–200k blocks); substitution recorded.* Budget: warm re-scan
        ~10–30 ms; if blown, facet indices move v2→v1. Running via codex
        (spec: `subagent-tasks/sheets-phase0-query-bench.md`).
  - [ ] Go/no-go + indices decisions recorded here.
- [ ] **Phase 1 — encoding + read-only positional grid** (render `tine.view:: grid`
      children as positional table; logical-matrix pass; §3.7 round-trip gate as
      an automated test EARLY; settle §13.2 mode-boundary + §13.4 header row as ADRs)
- [ ] **Phase 2 — editable cells + modality + full §4.3 keyboard** (largest phase)
- [ ] **Phase 3 — field-keyed table + query rowSource + task kanban** (showcase)
- [ ] **Phase 4 — Hierarchify/Flatten + board + aggregates**
- [ ] **Phase 5 — recursion + colors + polish**

## Decisions made (by Claude, per mandate)

- **ADR 0023** — render substrate: CSS Grid `max-content` tracks; `<table>`
  auto-layout rejected on Phase-0 numbers; no hard cell cap in v1, log-warn
  ~2000 cells.
- **ADR 0024** — §13.4 header row: explicit opt-in `tine.header:: true`, never
  auto-detected; positional face only.
- **ADR 0025** — §13.2 mode boundaries: click-on-text edits (mousedown entry,
  span→caret), click-on-whitespace selects cell, Esc ladder
  (edit→cell→outline), `←` past left exits, flow-out at top/bottom (no wrap),
  per-grid in-memory last-cell re-entry.
- **Spec §3.1 "Org coverage"** (Martin's Jul 6 question): Sheets is
  format-agnostic — org carries geometry as headline nesting and config as
  `:PROPERTIES:` drawer keys (`:tine.view: grid`); dotted keys verified legal
  in lsdoc's org drawer parser (rejects only `:`/space/newline, matches mldoc);
  org write-gating inherited from the existing round-trip self-check (failing
  pages read-only — no new rule). Every phase's fixtures include an org
  variant. Canonical md empty bullet = bare `-`, no trailing space.

## Decisions Martin should review

- All four above (each marked so in its ADR). The riskiest to taste is
  ADR 0025 (mode boundaries) — it's also the one the spec called most likely
  to feel wrong if rushed; revising it pre-merge is cheap.

## Sample pages for Martin

`~/research/tine-test/pages/Sheets demo.md` and
`~/research/org-graph/pages/Sheets demo.org` — one page each: positional grid
(header + ragged row), nested sub-grid, field-keyed table, task kanban
(`{{query (todo …)}}` + `tine.group-by:: state`) + scattered tasks for it.
Both validated round-trip-clean (md byte-identical; org passes
`org_round_trips` — checked with the new `roundtrip_org_dir` example). They
render as plain outlines until each phase lands.

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
