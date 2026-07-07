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
  - [x] (B) query — **GO; facet indices STAY v2.** Synthetic graphs (Martin's
        real graph unavailable + off-limits as corpus), release build, local
        disk; bench committed as `crates/tine-core/examples/sheets_phase0_bench.rs`
        (re-runnable, deterministic). Note: bare `TODO` isn't accepted by the
        simple DSL — `(task TODO)` is; compound = `(and (task TODO DOING) #SomeTag)`.
        | blocks | cold (incl. cache build) | edit→re-scan med/p95 | compound med/p95 | save_page |
        |---|---|---|---|---|
        | 10k | 12 ms | 0.6 / 1.1 ms | 0.4 / 0.9 ms | 0.5 ms |
        | 50k | 64 ms | 5.2 / 6.2 ms | 4.0 / 4.3 ms | 1.9 ms |
        | 100k | 130 ms | 9.5 / 11.8 ms | 8.2 / 8.8 ms | 4.8 ms |
        | 200k | 238 ms | 19.0 / 24.5 ms | 15.6 / 16.3 ms | 11.3 ms |
        Linear ≈1 ms per 10k blocks warm; the 10–30 ms budget holds through
        200k blocks (the tight 10 ms end frays ~100k p95). Memo hits ~0.1 µs.
  - [x] Go/no-go: **GO on both gates.** Auto-fit = CSS Grid max-content
        (ADR 0023); indices stay v2-if-measured; no hard cell cap.
- [x] **Phase 1 — encoding + read-only positional grid** — DONE Jul 6 2026
      (codex; verified: `npm test` 15 files/153 tests green both configs,
      `cargo test -p tine-core` 268 green incl. new md+org byte-exact grid
      fixtures in `tests/roundtrip.rs`, screenshot self-verified via
      `scripts/shot-sheets.mjs`). Shipped: `src/sheet/config.ts` (tine.*
      parser) + `src/sheet/matrix.ts` (logical matrix, holes, span-ready) +
      `src/components/SheetGrid.tsx` (read-only renderer: header row, holes,
      nested sub-grids depth-capped at 5, org-aware via `facetsOf(raw, fmt)`)
      + Block.tsx children-branch integration + `tine.*` chips hidden
      (render/block.ts prefix rule) + mock demo page. §13.2/§13.4 settled as
      ADR 0025/0024. **Phase-2 note:** a cell whose block is a task renders
      body-only — the TODO marker/priority are invisible in grid cells (seen in
      the screenshot); Phase 2/3 must render cell header-facets (marker,
      priority chips) inside cells, and remember BOTH facet renderers rule
      (`tine-refblocks-vs-block-facets`) now has a third surface.
- [ ] **Phase 2 — editable cells + modality + full §4.3 keyboard** (largest phase;
      split into sub-chunks)
  - [x] **2a — cell selection, editing, core keyboard** — DONE Jul 6 2026
        (codex + one orchestrator fix). Cell = another surface onto the block
        (`sheet:<gridId>` SurfaceContext key, scoped owner per cell); overtype
        flows through the real editor commit path (textarea input event, no
        store bypass); grids OPAQUE to outline nav (`visibleOrder` walkers skip
        grid interiors); mutual exclusion via new `src/modeHooks.ts` (store
        emits, sheet listens — no import cycle); `FORBID_EDIT_SELECTOR` moved
        to shared `src/editor/editTargets.ts`; in-cell editor blocks all
        structural commands (Enter=commit+reselect, Tab=move, no
        split/indent/merge). Keyboard: arrows 2-D, flow-out per ADR 0025,
        Enter/F2/Esc ladder, Tab row-wrap, printable overtype, outline-side
        Enter/→ entry. **Orchestrator fix on codex output:** its
        `sheetConfigFromRaw` hand-rolled md+org property scanners (duplicate
        recognizer — banned); rerouted through `facetsOf` (the one lsdoc
        recognizer); the real blocker was wasm init in node-config tests →
        `beforeAll(initParser())` in store/config tests. Verified: npm test
        45+16 files green, tsc clean, both screenshots eyeballed (selection
        ring on a hole cell).
  - [x] **2b — seams** — DONE Jul 7 2026 (codex + orchestrator fix).
        `withUndoUnit(tag, pages, fn)` in store.ts (one snapshot, nested pushes
        suppressed, rollback-on-throw, nested units join — unit-tested);
        `insertEmptyChildBlock` keeps produce-surgery inside store.ts;
        `src/sheet/mutations.ts` orchestrates insertRow/deleteRow/insertColumn/
        deleteColumn/materializeCell/setColumnWidth — column ops rewrite
        `tine.col-widths` indices in the SAME unit; `serializeColWidths` lives
        beside `parseColWidths` (one grammar owner). Seam selection (`SheetSel`
        row-seam/col-seam + companion index), arrow-stepping cell→seam→cell
        (module const `SEAM_STEPPING`, boundary seams then flow-out), type/
        Enter-on-seam inserts + edits, Backspace/Delete delete before/after,
        pointer seam click (±3px ruling hit) + column drag-resize (live
        preview, one property write on pointerup, dbl-click clears width).
        **Orchestrator fix:** boundary seams painted nothing — positioned
        exactly on the `.sheet-grid` overflow-clip edge; clamped into the
        content box (`seamStyleFor`). Verified: npm test 46+16 files green,
        tsc, interact tests 10/10, seam bar eyeballed in the cropped shot.
  - [x] **2c — ranges, content move, fill, clipboard** — DONE Jul 7 2026
        (codex, verified clean — no orchestrator fixes needed). `SheetSel`
        range variant (anchor/focus; cell = degenerate); Shift+arrow extend,
        Shift/Ctrl+Space row/col select, grid-scoped Ctrl+A;
        `.sheet-cell-in-range` render (O(visible)); Ctrl+arrow cell/range/row
        moves (all-or-nothing, holes materialize on entry); Ctrl+D/R fill
        (strips id::, own raw only); mod+c TSV + HTML table via `copyRich`
        (shared backend path), mod+x = copy+clear; document paste listener
        (sheet-mode only, editable-target guarded): TSV/CSV (quoted fields,
        tabs>commas, `src/sheet/tsv.ts` owns both directions) anchored paste
        that GROWS the grid, indented text via shared `parseOutline`
        (editor/outline), single line = replace raw. All gestures one
        `withUndoUnit`. v1 simplifications noted: ranges are single-level
        (nested grids ride as content — TreeSheets merge rule deferred), fill
        doesn't copy children. Verified: npm test 47/442 + 16/167 green, tsc,
        e2e ALL PASS, range shot eyeballed.
      **PHASE 2 COMPLETE — the grid face is fully usable.**
- [x] **Phase 3 — field-keyed table + query rowSource + task kanban** — DONE
      Jul 7 2026 (codex + 2 orchestrator fixes). `src/sheet/fields.ts` = the
      Field abstraction (state/priority/scheduled/deadline/tags/page/prop:<k>),
      reads facets, writes route through marker (`setMarker` added beside
      `cycleMarker`) / planning / `setBlockProperty`; tags+page read-only v1.
      `SheetTable` (CSS Grid, row-title col, field columns, view-only sort,
      state/priority click-cycle, prop inline input, children-only column-add
      per §3.8) + `SheetBoard` (own grouped renderer, NOT the matrix; state/
      priority/prop axes + `(none)`; card counts; facet-chip card faces;
      pointer drag + Ctrl+←/→ card-move = single-block field write). Query
      source: both faces branch inside QueryMacro reusing its `groups()`
      resource; unloaded rows render DTO facets read-only until the page loads.
      Rust: `tags` facet added to BlockProjection/DTO off the one lsdoc
      projection (ADR 0009 pattern). Slash commands /grid /table /board.
      **Orchestrator fixes:** (1) codex's `setBlockProperty` rewrite hoisted
      ALL property-looking lines from anywhere in the body to the head —
      fence-unaware reordering = data hazard; rewrote to scan ONLY the
      canonical head region (planning→props) + the legacy trailing prop block,
      never the body; regression tests pin fence safety, OG line order
      (planning before props), and legacy-tail cleanup. (2) mock's tag regex
      leaked `[#A]` priority as a `#A` tag (screenshot caught it; the real
      lsdoc extractor verified clean by probe) — lookbehind fix. Verified:
      npm 48+18 files green, tsc, cargo 270 (incl. new md+org table/board
      round-trip fixtures), table+board shots eyeballed, e2e ALL PASS.
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
