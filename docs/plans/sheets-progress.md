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
- [x] **Phase 4 — Hierarchify/Flatten + aggregates** — DONE Jul 7 2026 (codex,
      verified clean). `src/sheet/restructure.ts`: hierarchify(parent, field)
      groups children by field value into new first-seen-order group blocks
      (one withUndoUnit); flatten pulls grandchildren up, deletes emptied
      groups, writes labels back only for writable round-tripping fields the
      row lacks (group-by config → inferred; unknown property keys = no
      write-back, documented). Inverse-pair + undo + no-op tests. Context
      menus: Hierarchify-by submenu + Flatten (children-source only), board
      "Hierarchify into columns". `src/sheet/aggregate.ts`: Bases v1 set
      (+count), skipped-count display; footer selector row on table+grid;
      `tine.col-aggregates::` (field ids / positional indices) parsed+
      serialized beside col-widths; column insert/delete shifts BOTH configs
      in one unit; values derived, never stored. Safe guard added to
      deleteBlockInternal (detached-block tolerance). Verified: npm 50+18
      green, tsc, cargo 270 (new fixtures), shots + e2e ALL PASS.
      **Phase-5 polish note:** footer is always-visible None-dropdowns —
      should collapse to values/dimmed Σ until hover.
- [x] **Phase 5 — v1 completion & polish** — DONE Jul 7 2026 (codex, verified
      clean). Shared `src/blockColors.ts` (block + cell menus, one write
      path); cell context menu (face switch via `tine.view` on the cell,
      zoom-into-cell, colors) + hover-reveal ⋮ handle (menu-only; pointer
      cell-drag deferred to v2); aggregate footer collapsed to quiet
      values/hover-Σ (no layout jump); scheduled/deadline cells edit via the
      existing DatePicker through `writeField`; **tag-page table**: opt-in
      `tine.tag-table:: true` toggle replaces linked refs with a query-sourced
      field table over `(tag <page>)` (`tag` added as additive alias of
      `page-ref` in the Rust DSL) + **add-row → today's journal** with the
      pre-filled tag (decided: scattered instances have no home page);
      onboarding `[[Features/Sheets]]` template + website/demo regenerated
      (static demo renders sheet blocks as plain outline — correct, it's the
      round-trip story); FEATURES.md rewritten to the shipped list;
      `docs/img/sheets.png` + SCREENSHOTS.md row + README image. Verified:
      npm 50+19 green, tsc, cargo 270, all shots eyeballed, release binary
      rebuilt, e2e ALL PASS.

## Post-v1 hardening pass (Jul 7 2026, same day)

Two independent adversarial reviews over the whole branch diff (codex xhigh +
an Opus subagent; brief: `subagent-tasks/sheets-v1-review.md`, findings in
`subagent-tasks/notes/sheets-v1-review-{codex,opus}.md`), plus an extended
real-app e2e (seams, fill, undo, board card-move — 18 checks, ALL PASS).
Master was merged INTO the branch (master had moved: GH #25 org id-drawer fix
+ backlog edits; the merge brought the drawer machinery the org fix builds on).

**Fixed (all with regression tests):**
1. (P1, Opus, validated) `writeField` state-cycle double-applied the
   timetracking transition → duplicate never-closed `CLOCK:` on every kanban
   card move. Cycle branch now passes `{timetracking:false}`.
2. (P1, codex) `setSchedule` deleted `SCHEDULED:` lookalikes anywhere in raw
   (incl. code fences; pre-existing bug, newly UI-reachable via date cells) —
   now edits only the canonical head region.
3. (P1, codex) Sheet faces bypassed the org read-only gate — gated at the
   chokes: `gridPage` (all structural mutations), `writeField`/`cycleField`,
   `startCellEditing`, hierarchify/flatten.
4. (P2, both) `setBlockProperty` wrote markdown `key::` lines into org blocks —
   org branch now writes the `:PROPERTIES:` drawer (canonical title→planning→
   drawer→body placement, mirrors master's `rawWithBlockId`); empty drawer
   removed on last-property delete.
5. (P3, codex) `blockProperty` reads now go through `facetsOf` (fence-safe,
   case-insensitive) instead of a raw regex scan that could suppress config
   writes.
6. (P3, Opus) grid column insert/delete no longer drops field-keyed
   `tine.col-aggregates` entries (positional index shift preserves them).
7. (P3, Opus) cell clear/fill/cut preserve hidden `id::`/`collapsed::` (no
   more dangling ((refs)) after clearing a referenced cell).
8. (e2e-found, NEW) board card mousedown bubbled into the query block's edit
   gesture → unscoped edit of the {{query}} block stomped card editing;
   stopPropagation like grid cells.
9. (e2e-found, NEW) a query block with `tine.view:: board`/`table` rendered a
   SECOND empty children-source board under the query-source one (phantom
   columns); Block.tsx now defers those faces to the query macro.

**Known, deliberately not fixed (small, noted for Martin):**
- `cutSheetSelection` clears before the async clipboard write settles (a
  rejected write loses the cut text; undo recovers it).
- `setBlockProperty` can't remove a property that IS the block's first line.
- Hierarchify group labels like `TODO` render as task blocks in OG (bare
  label text — cosmetic, round-trip safe).
- Logseq's default workflow is LATER/NOW: a config-less graph shows those
  board columns, not TODO/DOING (OG parity, surprised the e2e author).

**SHEETS v1 COMPLETE + HARDENED (Jul 7 2026) — all §11 phases done, branch `sheets`.**
Remaining for v2 (per spec §10): pointer cell drag; merged cells `span::`;
formulas (Bases DSL); multi-valued group-by (tag boards); `tine.fields::`
schema; split view; canvas face + whiteboards importer; in-column card
reorder; tags/page field write-back; multi-level (cross-hierarchy) ranges.

## Decisions made (by Claude, per mandate)

- **ADR 0023** — render substrate: CSS Grid `max-content` tracks; `<table>`
  auto-layout rejected on Phase-0 numbers; no hard cell cap in v1, log-warn
  ~2000 cells.
- **ADR 0024** — §13.4 header row: explicit opt-in `tine.header:: true`, never
  auto-detected; positional face only.
- **ADR 0025** — §13.2 mode boundaries: click selects a cell, double-click edits
  with span→caret mapping, Esc ladder
  (edit→cell→outline), `←` past left exits, flow-out at top/bottom (no wrap),
  per-grid in-memory last-cell re-entry.
- **ADR 0026** — Phase-6 field schema: `tine.fields:: name=type;…` scalar
  grammar (text/number/date/datetime/checkbox/list/enum/ref + built-ins as
  `state=state` for ordering); homes = view block's props OR the tag page's
  page properties (view wins); declared-first stable columns, strays italic
  and never hidden; canonical stored forms checkbox `true`/`false`, dates ISO;
  add-row seeds tag-only (no empty `key::` junk).
- **ADR 0027** — Phase-6 tags write-back: delta-shaped (one tag add/remove per
  call), FIRST LINE only, removal cut via the lsdoc Tag-inline span (never
  regex); tag boards = Notion model (card in every matching column; move =
  remove+add as one undo unit; `(none)` accepts drops only from single-valued
  cards).
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

## Phase 6 — the supertag/database layer (IN PROGRESS)

Plan: [sheets-phase6-plan.md](sheets-phase6-plan.md). Decisions settled as
ADR 0026 (schema) + ADR 0027 (tags write-back / tag boards).

- **6a DONE (Jul 7)** — schema core: `tine.fields::` parse/serialize in
  `src/sheet/config.ts` (`parseFields`/`serializeFields`, malformed-tolerant,
  scalar-safe); schema resolution own-props → `schemaPage` page property
  (TagPageTable passes its page); declared-first column ordering with
  always-present declared columns + italic `sheet-col-stray` strays;
  read-side typed rendering (checkbox/number-align/date badge/enum + list
  chips/ref via InlineText); board enum column ordering (declared values
  first, `(none)` always for enum group-by). Codex build + 4 orchestrator
  verifications (both vitest configs 479+190, tsc, cargo, DOM header-order +
  stray-class probe, screenshots incl. scrolled right edge). Mock demo table
  gained a schema. Zero-schema rendering verified unchanged (regression
  test). Deviation, per one-parser rule: query-block tables are
  own-props-only (no tag-page fallback — Macro.tsx has no parsed tag handy
  and a query-string regex was forbidden); revisit only if a real parsed tag
  target appears.
- **6b DONE (Jul 7)** — typed editing + add-row + header schema menu:
  checkbox cells click-toggle (`true`/`false`, rendered box stays disabled +
  `pointer-events:none`); date/datetime cells reuse THE date picker
  (`DatePickerTarget` union — prop targets write ISO, datetime preserves the
  `HH:MM` tail, repeater UI schedule-only); enum cells open a values popup +
  Clear (new generic `action-menu` context-menu variant, reused for the
  header menu); number inputs validate (`sheet-input-invalid`, invalid
  commit keeps the editor open); children-table add-row (one undo unit,
  opens title cell); header right-click menu: declare stray / first-declare
  seeds the FULL observed order (no reshuffle) / change type / remove —
  writes to the effective schema home (block prop or tag-page page prop),
  read-only gated. Orchestrator fixes over codex: blur-refocus used
  `e.currentTarget` inside `queueMicrotask` (null after dispatch — throw
  instead of refocus; fixed + regression test). E2E extended (21 checks ALL
  PASS vs the real binary: checkbox → `shipped:: true` on disk, enum popup,
  enum pick → `topic:: ui` on disk); seed grew so two bullet-count
  expectations were re-based (9→11, 11→13). Deviations (deliberate): list
  chip editor out of scope (comma text editing); enum types are hand-edit
  only in the header menu. Sample pages (tine-test md + org-graph org)
  gained a playable "§6 typed schema table" — both graphs re-validated
  round-trip clean. NOTE: `~/research/tine` still runs the v1 binary
  (ae9bb81) on purpose — Martin's nit list is against v1; deploy on request
  or at phase end.
- **6c DONE (Jul 7)** — tags write-back + tag boards (ADR 0027, amended:
  org tags are headline `htags`, no inline spans → write-back is
  markdown-only; org tag boards render, moves refuse): `writeTagDelta`
  (delta API, one `withUndoUnit` + one `setRaw`; add appends the shared
  `tagRef` token to line-1 end; remove cuts the lsdoc tag-inline span via
  the click→caret span mapping — `parseBody` +
  `rebulletedSourceByteToRawByte`, NO regex; body-line-only /
  code-span / org / read-only all refuse); board `tine.group-by:: tags` =
  Notion model (card in every matching column via `groupKeysForBlock`,
  `(none)` for zero-tag rows, drag identity now (id,col,row) so duplicated
  cards drag right, target column re-found BY KEY after re-group);
  moves on both paths (drag + Ctrl+arrow): none→B add, A→(none) only
  single-valued, A→B remove+add. Codex also consolidated the THREE tag-token
  formatters (Page.tsx tagRef, autocomplete tagInsert, fields add) into
  `src/tags.ts`. Orchestrator fix: the consolidated rule over-bracketed
  unicode (`#čeština` → `#[[čeština]]`); rewrote `tagRef` parser-aware from
  lsdoc's actual TAG_STOP set (bare unless whitespace/`#,!?'":`/brackets/
  trailing `.;`) + unit tests — Czech tags stay bare, and it fixes latent
  bugs in BOTH old rules (old tagInsert emitted broken `#a,b`; old tagRef
  emitted `#trail.` which parses short). E2E now 24 checks ALL PASS (2-tag
  card in both columns; on-disk move `#alpha`→`#beta` exact-token; sibling
  untouched); the old "exactly one board" regression guard re-scoped by
  content (the seed now legitimately has two boards). Known-minor: tag
  COLUMNS key by exact string, so `#ChoCo` vs `#choco` across rows makes
  two columns (removal itself matches case-insensitively; OG treats tags
  case-insensitively as pages) — revisit if it bites. Sample page gained a
  playable tag-board §7 (round-trip re-validated).
- **6d DONE (Jul 7) — PHASE 6 COMPLETE** — conversions + CSV drop + docs:
  `src/sheet/conversions.ts`: pipe-table→grid (block context menu, offered
  only when the body parses to leading blocks + exactly ONE trailing table;
  cell raws sliced from source spans so `**bold**`/`[[refs]]` survive;
  header row → `tine.header:: true`; aligns dropped; refuses when the block
  already has children); grid→pipe-table (grid context menu; refuses every
  lossy case with a specific toast — non-empty row raw, hidden props,
  multiline/child-bearing cells, >30×200, and `|`-containing cells because
  a RUNTIME PROBE against the real parser shows lsdoc does not round-trip
  `\|` in table cells — the probe auto-upgrades if lsdoc ever does); both
  directions one undo unit, byte-round-trip tested. CSV/TSV file-drop →
  grid via the existing `tsv.ts` owner (extension-selected delimiter, 5000-
  cell cap, one undo unit) reading through a NEW Tauri command
  `read_text_file`. Docs synced: FEATURES, README bullet+pitch, CHANGELOG
  Unreleased "Added", onboarding sheets.md template (typed table + tag
  board), website/demo regenerated, SCREENSHOTS.md row, docs/img/sheets.png
  refreshed; mock demo gained a tag board. Orchestrator fixes over codex:
  (1) `read_text_file` took ANY absolute path from the webview — the only
  ungated read in the IPC surface; extension-gated to .csv/.tsv with a
  don't-grow-this comment; (2) grid→table rebuilt the host from
  first-line+facets, silently DROPPING body lines — replaced with
  strip-tine-props-only (verbatim preservation) + regression test;
  (3) added a no-title-line refusal (setBlockProperty can't remove a line-0
  property — known v1 limitation — so stripping would half-convert);
  (4) reverted codex's drive-by `cargo fmt` noise across 8 unrelated Rust
  files. E2E re-run on the rebuilt binary: 24 checks ALL PASS. Sample page
  gained a §8 pipe table to convert (round-trip re-validated).

**Phase-6 adversarial review (Jul 7, post-6d)** — two independent
validating reviewers (codex xhigh + Opus subagent), whole-subsystem scope,
every finding reproduced by the orchestrator before fixing. ALL FIXED same
day (regression tests in `src/sheet/review-regressions.test.ts` +
`fields.test.ts`; e2e re-run ALL PASS):
- **P1 (codex)**: OS file-drop could mutate READ-ONLY pages —
  `insertOutlineAfter` had no gate; now refuses at the choke point (covers
  every caller).
- **P1 (codex)**: the aggregate footer's `setColumnAggregate` bypassed the
  `gridPage` read-only gate and wrote `tine.col-aggregates::` onto
  read-only owners; gated.
- **P2 (codex)**: tag-page add-row into an EMPTY today journal pushed three
  undo entries (anchor/insert/delete) — one undo left an empty anchor + the
  row behind; the empty-page append path is now one `withUndoUnit`.
- **P2 (opus)**: `writeTagDelta` removed only the FIRST occurrence of a
  duplicated first-line tag and reported success (board move fails open);
  now cuts until gone.
- **P3 (opus)**: four hand-rolled ISO-date recognizers (fields/aggregate/
  SheetTable/DatePicker) consolidated into `typed.ts`
  (`parseIsoDateLike`/`isoDatePrefix`) — one-parser rule restored.
- **P3 (opus)**: `read_text_file` now also canonicalizes and re-checks the
  extension on the RESOLVED path (symlink named `x.csv` can't dodge the
  gate).
Both full reports: `subagent-tasks/notes/sheets-phase6-review-{codex,opus}.md`
(local). Notable: both P1s were v1-era paths — the whole-subsystem scope
rule (not diff-only) is what caught them.

## Phase 7 — formula columns (IN PROGRESS)

Plan: [sheets-phase7-plan.md](sheets-phase7-plan.md); DSL decided in ADR 0028.

- **7a DONE (Jul 7)** — the pure expression engine, `src/sheet/formula/`
  (lexer + Pratt parser with offset-carrying parse errors, typed evaluator
  with errors-as-values, stdlib per ADR, duration/date math with month-end
  clamping, DAG cycle errors naming the chain, injected-`now` purity, and
  the property-line armor: `((`→`( (` add-one-space/remove-one-space
  inversion + `\#` escape-the-escape inside strings). Verification: purity
  grep (no store/ui/Date.now imports), full 13-op × 7×7 coercion-matrix
  test, 500-case seeded fuzz (print→parse identity + encode/decode fixed
  points with `#`/`((`/quote/backslash strings), armor hand-analyzed;
  531+204 tests, tsc, cargo all green. Grammar notes: numeric method
  receivers need parens (`(2).round()`); `.length`/`.year` are
  property-style members. Nothing user-visible yet (engine only).
- **7b DONE (Jul 7)** — computed columns: `formulasOf` collection
  (`tine.formula.<name>` prefix, decode armor, view-over-page merge per
  name), `formula:<name>` FieldId (read-only, `readField`/`writeField`
  refuse), `formulaEval.ts` store-boundary (facet→FormulaValue conversion
  table, Solid-memo results keyed on dataRev + row raws per ADR 0014,
  bounded parse cache, 10k-evals warn), SheetTable renders ƒ-marked
  columns after declared/before strays with typed rendering by result
  type + ⚠ error chips (message on hover), sort + footer aggregates work,
  header menu offers Remove formula. Orchestrator fixes over codex:
  (1) scheduled/deadline facets ("2026-07-08 Wed") coerced to TEXT, which
  broke `deadline < today()` — now isoDatePrefix→date (+ regression
  tests); (2) unbounded module parse cache bounded at 500. Probe-verified
  in the browser: `points * 2` → 6/16/2, `due-soon` boolean checkboxes
  correct per row, parse-error column renders ⚠ chips carrying the
  message. A suspected crash (referenced formula with a parse error
  thrown from inside evaluate) probe-checked: NOT real — the engine
  already converts it to an error value. E2E ALL PASS; deployed to
  `~/research/tine-sheets`; tine-test gained a §9 formulas section
  (round-trip re-validated).
- **7c DONE (Jul 7)** — formula axes + filter + editor:
  `tine.group-by:: formula.<name>` (dot form → `formula:` field id;
  boolean buckets true,false; `(none)`/`(error)` last; moves REFUSED on
  formula axes — drag affordance off + both paths gated);
  `tine.filter::` on tables AND boards via a shared
  `createFormulaFilterMemo` (fail-OPEN honesty rule: parse error or any
  non-boolean/error row disables filtering entirely + a
  `sheet-filter-error` chip carries the message; aggregates/counts use
  filtered rows); root-mounted `FormulaEditor` (DatePicker pattern):
  add/edit formula + filter mode, LIVE parse validation with caret
  marker, field/formula/stdlib chips, save disabled until valid, one
  property write via the schema-home rule, read-only gated.
  Verification: gates green (539+214, tsc, cargo); browser probe (menu →
  editor → live error + disabled save → valid filter save → rows 3→2);
  e2e extended to **30 checks ALL PASS** incl. real-app add-formula
  (property on disk, computed value NOT on disk, column renders).
  Deployed to `~/research/tine-sheets`; tine-test gained the 7c try-it
  note + a §10 filtered-table section (round-trip re-validated; NOTE:
  Martin is actively editing that graph while testing — always re-read
  before editing).
- **7d DONE (Jul 7) — PHASE 7 COMPLETE** — docs pass: FEATURES.md gained
  the full formula/filter/editor section (syntax examples, stdlib list,
  honesty rule spelled out), README bullet + pitch, CHANGELOG Unreleased
  Sheets entry extended, onboarding template gained a formula line +
  website demo regenerated, SCREENSHOTS.md triggers updated,
  docs/img/sheets.png refreshed. Orchestrator fix: codex's README roadmap
  edit dropped "configurable typographic auto-replace" (still genuinely
  planned) and invented an "iOS" entry (redundant with the Mobile
  paragraph) — restored; its removal of the advanced-Datalog item was
  correct (shipped, CHANGELOG confirms). tine-test §10 filter sample
  re-applied after the concurrent-save clobber (Martin's app was holding
  the page) and re-validated; binary rebuilt with the new onboarding
  template and deployed to `~/research/tine-sheets`.

**Phase 7 leftovers** (deliberate): org-graph sample has no formula
section yet (Martin actively testing; add on request); custom
`values`-expression footer summaries; formula rename in the editor
(remove + re-add works); `.relative()`/`.format()` token set is minimal.

**Phase 6 leftovers for a later pass** (deliberate): list chip editor
(comma-text editing works); enum types are hand-edit-only in the header
menu; tag columns key by exact string (`#ChoCo`≠`#choco` as columns, though
removal matches case-insensitively); query-block tables don't inherit a tag
page's schema (no parsed tag target at the call site); org tags write-back
(htags-shaped, own decision needed); lsdoc `\|` table-cell fidelity (escaped
pipes don't round-trip, so `|`-cells refuse conversion — an lsdoc/oracle
question, the runtime probe auto-upgrades when fixed); planning
time-of-day dropped when a SCHEDULED/DEADLINE is rewritten from a sheet
date cell (needs `setSchedule` time support).

Martin's v1 UX nits are PARKED (his list, not yet captured) — batch later,
don't interleave.

## Round 5 — split view (Martin, Jul 8) [BUILD COMPLETE — S1–S4 ALL SHIPPED + deployed; awaiting Martin's daily-driving]

**S4 (Jul 8): pointer tab drag** (drop on strip = reorder/adopt, pane body
= move, seam/edge = split+move, Esc cancels; last HTML5-DnD site deleted),
**pane-scoped Ctrl+F**, docs (CHANGELOG/README/FEATURES/BACKLOG).
Supervisor fix: Ctrl+click-created panes now end with a SINGLE tab
(duplicate navigated in place, matching the embryo flow). Real-app probes:
split 10/10, nav 9/9, tab-drag 3/3, e2e 50/50. Probes live in
subagent-tasks/probe-{splitview,panenav,tabdrag,auxclick,auxclick2}.mjs.
NEXT: Martin's split-view nits → batch; then Round 6 = whiteboards-lite
canvas face. Deferred from Round 5 (spec §5): N-feed decoupling (two
journals panes), PDF/sidebar as pane leaves, stacked/ephemeral tabs,
named layouts, per-pane zoom. Note: no split-view screenshot yet — the
screenshot harness is mock-backend; take one via the real-app probes'
saveScreenshot if README wants it (WebDriver screenshots can hang after
edit sequences — known quirk).

**S3 (the nav model) shipped Jul 8:** pane-select as the top Esc rung,
spatial pane/seam/edge stepping (pure tree geometry), type-or-Enter on a
seam/edge → embryo split with prefilled QuickSwitcher (cancel unsplits),
mod+1..9 / mod+alt+arrows / mod+alt+shift+arrows (move tab via adoptTab),
geometric nearest-pane targeting, origin-pane-scoped block selection.
Keyboard-walk probe 9/9. Verification caught a stale-mode hazard: nothing
exited pane-select on pointer interaction and the dispatch branch ran
before the editing guard — entering the mode then clicking into a block
swallowed ALL typing; fixed (pointerdown exits + !editing guard,
fails-first test).

**S2 (two panes work) shipped Jul 8:** PaneTree renderer + pointer seam
resize; page panes load satellite-style (editing survives sibling
navigation); one journals feed pane (interceptor dedup); split right/down
`mod+alt+\` (duplicates current tab per §3a) + close pane; Ctrl+click +
switcher Alt+Enter = open in other pane; focused-pane accent; session
layout tree in src/session.ts (legacy-compatible both ways); app-level
watcher; eviction pins; mobile single-pane guard. Real-app split probe
10/10 + e2e 50/50. **Verification caught a real regression codex shipped:**
`endEditForSurface` inside PageView's loader effect subscribed the loader
to `editingId` — every startEditing re-ran it and instantly killed the
edit (all sheet cell editing dead in the real app, invisible to jsdom).
Fixed with a load-bearing `untrack()`. Plus the frozen-binding family
(solo-pane topbar strip, keyed PaneLeaf remount, static surface keys).

**S1 (pane-router extraction) shipped Jul 8:** zero-behavior refactor, all
gates + e2e 50/50 + two real-app probes green (capture-phase pointerdown
precedes middle-click handling — the shim-design assumption — and the shim
path opens background tabs correctly); binary deployed. Three S2 landmines
found in verification and folded into the S2 spec
(`subagent-tasks/split-s2-two-panes.md`): per-instance session clobber,
`zoomedBlock()` via focused-pane shim, per-instance mobile popstate bridge.
Open-target rulings (Martin's three usability questions) = spec §3a.

**GO confirmed by Martin ("Ok - go for the split screen"). Spec =
`docs/split-view-spec.md` (phases S1–S4); architecture facts =
`subagent-tasks/notes/split-view-architecture.md`; decision spine = ADR 0032
(shape A: pane-router factory + PaneContext + focused-pane shims; single
journals feed pane v1; layout in tine-session.json).**

Martin (Jul 8): wants BOTH deferred v3 tracks unless a new reason against
emerged — (1) whiteboards-lite (free-floating cards) and (2) split screen
with the grid nav model (à la TreeSheets: typing at an edge opens a new
split). **He's more excited about split screen** — it goes first.

Assessment (no new reasons against; the picture IMPROVED in rounds 1–4):

- **Split view** (`leafKind: pane`, spec §10 v3): the hard UX half — the
  seam-as-selection-target machinery, edge-typing materialization, the
  select→edit ladder — now EXISTS and is daily-driven in the grid. What
  remains is the known cost, unchanged since the spec: per-pane
  `RouteContext` instead of the singleton `route()`/tabs in `src/router.ts`,
  a pane split-tree (workspace state, NOT document — must persist via the
  Rust backend, WebKitGTK localStorage is ephemeral), pane chrome, and
  pointer-events-only seam/tab drag (never HTML5 DnD).
- **Whiteboards-lite** = the canvas face (`trackModel: free`, spec §10 v3,
  Jul 5 decomposition): cards + `tine.x/y/w/h` scalar props + ref edges,
  round-trips as a flat bullet list. No new reason against; one thing got
  BETTER: master's #38 media-editor registry (drawio/Excalidraw
  edit-externally) just merged in, so the freehand-ink job is served
  outside the engine and the canvas face stays cleanly cards-only.
  Queued as Round 6, after split view.

## Working notes

- **Deploys (Martin, Jul 7): Sheets builds go to `~/research/tine-sheets`**;
  `~/research/tine` stays on the master line so he can review master fixes
  in parallel. Deploy after each verified chunk. Keep the tine-test
  `Sheets demo` page current with each new feature (round-trip-validate
  after edits).

- **Branch hygiene:** the checkout had pre-existing uncommitted edits from
  another session (`docs/plans/theme-gallery.md`,
  `src-tauri/gen/schemas/acl-manifests.json`) — left untouched and uncommitted;
  never `git add -A` / `commit -a` / stash / checkout / clean on this tree.
  The BACKLOG.md Flathub hunk (Martin's Jul 6 decision) was committed together
  with the Sheets "Now" entry so it isn't lost.
- Commits on this branch: push with `git push origin sheets` after each chunk.
