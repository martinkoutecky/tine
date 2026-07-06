# Plan — Queries batch (Next #1): aggregation, discoverable-advanced, coverage

**Status:** approved, not started (Martin promoted it to Next, Jul 6 2026). This doc is
the resume-from-cold spec — it captures the subsystem map, the decisions, and the ordered
steps so the work survives a context compaction. Build in the order below.

Three features, one subsystem:
- **1a Aggregation on query results** — no-code count / sum-of-property / average / group-by.
- **1b Discoverable advanced `[:find …]` queries** — a "switch to advanced" affordance.
- **1c Datalog coverage expansion** — widen the advanced clause subset.

## Subsystem map (verified Jul 6 2026)

**Engine — `crates/tine-core/src/query.rs`:**
- `enum Pred` (~1143) mirrors the builder clauses: `PageRef, Task, Priority, Property,
  Scheduled, Deadline, Journal, Between(BetweenField,…), Page, Namespace, PageProperty,
  PageTags, Content, And/Or/Not, Sample(usize), SortBy(String,bool)`. `Sample`/`SortBy`
  are **result-level options** whose `eval` returns `true` (no filtering) — the template
  to copy for an `Aggregate` directive.
- Parse: `Pred::parse` (~1250) bails to `None` if `is_advanced(src)` (~1134), else
  `tokenize` (~1407) → `parse_expr` (~1486). Clause-head dispatch = `match head` at
  **1508–1599**; unknown head → `return None` (**1599**) → **whole query fails**. (Critical
  constraint for 1a — see decision below.)
- Eval: `Pred::eval(block, ctx)` (~1271); `EvalCtx` (~1195) carries page facts.
- Results: `run_query`(~231)→`run_pred`(~243). Returns **`Vec<RefGroup>`**, NOT ids.
  `RefGroup { page, kind, blocks: Vec<BlockDto> }` (model.rs 191). Each `BlockDto`
  (model.rs 159, built by `block_to_dto` model.rs ~3543) already carries
  `marker, priority, scheduled, deadline, properties: Vec<(String,String)>`. `sample N`
  truncation ~337.
- Advanced: `is_advanced` (~1134), `run_advanced_query` (~445), `parse_adv_group`
  (**538–621**) — supported heads today: `and/or/not, task, priority, page-ref, property,
  page-property, between` (between hardwired to `BetweenField::Journal` at ~612). The
  **simple** parser supports strictly MORE (`page, namespace, page-tags/tags, scheduled,
  deadline, journal, sample, sort-by`). Returns `AdvancedResult { groups, ran, ignored,
  supported }` (~431) — ran-vs-ignored already reported.

**Command boundary — `src-tauri/src/commands.rs`:** `run_query`(238)→`Vec<RefGroup>`,
`run_advanced_query`(246)→`AdvancedResult`, `query_facets`(257)→property facets for the
builder pickers. JS: `backend.ts` `runQuery`(97)/`runAdvancedQuery`(100)/`queryFacets`(103);
types in `src/types.ts` (`RefGroup`/`BlockDto`/`AdvancedQueryResult` ~146–159); mock in
`src/mock.ts` (`runQuery` 465, `runAdvancedQuery` 460, `queryFacets` 526).

**Builder model — `src/editor/queryBuilder.ts`** (DOM-free; SSOT is the DSL text):
`type Clause` (~14), `parseQuery`(~325), `toDsl`(~405)/`clauseDsl`(~357), mutations
(~542), `SORT_PRESETS`(~430), `clauseLabel`(~450).

**Builder UI — `src/components/QueryBuilder.tsx`:** `QueryBuilder({dsl,onChange,blockId})`
(~170) → `apply(next)=onChange(toDsl(next))` (~184); `SortControl` (69–167) is the model
for a result-level control; `FILTER_TYPES` (378–392) registers clause kinds; `AddPicker`
(394), `ValuePicker` (437), `ChipMenu` (296). Auto-open signal in `src/ui.ts` (~368),
triggered from `Block.tsx` (~1241).

**Render — `src/components/Macro.tsx` `QueryMacro`** (~70): `ADVANCED_RE`(14)/`isAdvanced`
(125) picks the path; `createResource` fetch keyed on `dataRev` (142–155); `total()` (156)
= working count; `rows()` (166–176) flattens groups to `{page,kind,text,props}`; `cols()`
(178); table sorts by property client-side (184–189); header `query-count` (284); result
body `<Switch>` (~210). `advInfo` (ran/ignored) already surfaced for the advanced path.

**Tests:** `src/editor/queryBuilder.test.ts` (`roundtrip = toDsl(parseQuery(dsl))`, ~16 +
per-clause suites) is the builder oracle. `query.rs #[cfg(test)]` (~1672): `pred(src)`
helper, parse/eval tests, `advanced_datalog_is_unsupported` (~1762).

## Decisions (locked)

- **D1 — aggregation is computed in JS, from the returned block list.** The DTO already
  ships every field an aggregate needs; `rows()` already flattens them; `total()` already
  counts. No Rust aggregate payload, no second render branch. (Scout-confirmed: lower risk,
  no duplication, no perf win from Rust since the set is already materialized + bounded.)
- **D2 — the aggregate directive lives IN the `{{query}}` DSL, and Rust must parse-but-
  ignore it.** The builder's single source of truth is the DSL text, and `{{query}}` must
  round-trip. But `parse_expr` fails the whole query on an unknown head (1599), and
  `Macro.tsx` sends the DSL to `run_query`. So add an `Aggregate` **no-op-filter** `Pred`
  variant (eval→`true`, exactly like `Sample`) parsed from `(aggregate count)`,
  `(aggregate sum <prop>)`, `(aggregate avg <prop>)`, `(group-by page|<prop>)`. Rust then
  returns the full set; the frontend re-parses the same DSL (`parseQuery`) to know which
  aggregate to render. This keeps run_query succeeding and the builder round-tripping.
- **D3 — OG-parity flag (surface to Martin when shipping).** OG does aggregation only via
  datalog `:result-transform`/`(count …)`, which Tine lists as `ignored`. A no-code
  JS aggregation is a **Tine-specific addition on top of OG**, not OG-faithful — note the
  divergence (per the match-OG working agreement) but it's an explicitly-wanted beat-OG.
- **D4 — 1b emits a datalog skeleton**; because `Macro.tsx` gates on `ADVANCED_RE`, writing
  `[:find …]` text auto-flips the render path to `runAdvancedQuery`. The skeleton lists the
  **supported** heads (post-1c) as inline hints; `advInfo` already shows ran/ignored so
  mistakes stay visible.

## Ordered steps

**Step 1 — 1c (coverage expansion) first** (unblocks the 1b skeleton's hint list):
- Widen `parse_adv_group` (query.rs 556–620) to the heads the simple parser already has but
  advanced lacks: `page`, `namespace`, `page-tags`/`tags`, `scheduled`, `deadline`,
  `journal`; make `between` field-aware (drop the hardwired `BetweenField::Journal` at ~612,
  read the field like the simple parser does).
- Tests: extend `query.rs` tests next to `advanced_datalog_is_unsupported` — assert each new
  head maps (appears in `ran`, filters correctly) and unknowns still land in `ignored`.

**Step 2 — 1a (aggregation):**
- Rust: add `Aggregate` (and/or reuse for group-by) variant to `Pred` (~1143); parse heads
  `aggregate`/`group-by` in `parse_expr` (~1508); `eval`→`true` (no filter, like `Sample`
  ~1327); make sure `collect_opts`/`run_pred` don't treat it as a filter. Round-trip test in
  `query.rs`.
- Builder model (`queryBuilder.ts`): add an `aggregate`/`group-by` `Clause` kind; parse in
  `parseQuery`/serialize in `clauseDsl` so `toDsl(parseQuery(dsl))` round-trips; add
  `clauseLabel`. Round-trip tests in `queryBuilder.test.ts`.
- Builder UI (`QueryBuilder.tsx`): a result-level control modeled on `SortControl` (69–167)
  — a dropdown (None / Count / Sum property / Average property / Group by page|property),
  the property choices from `queryFacets()`. Render it next to `SortControl` in the
  `QueryBuilder` return (~190).
- Render (`Macro.tsx`): read the aggregate directive (re-parse `form()` via `parseQuery`, or
  thread it from the builder); compute a `createMemo` over `rows()` (166) — count = length;
  sum/avg = parse the chosen property's values as numbers (skip non-numeric, surface how many
  skipped); group-by = a `Map<groupKey, rows[]>` rendering one `QueryGroup`/table per key.
  Render count/sum/avg near the header `query-count` (284) or as a summary row above the
  `<Switch>` (210); group-by replaces the flat list with per-group sections.
- Mock: ensure `src/mock.ts runQuery` returns blocks with `properties` so the aggregate is
  demoable in the screenshot harness; add a fixture query.

**Step 3 — 1b (discoverable advanced):**
- Add a "Switch to advanced" button to `QueryBuilder.tsx` (~190) → `props.onChange(skeleton)`
  where skeleton = `[:find (pull ?b [*])\n :where\n  ; supported: (task TODO DOING) (priority A)
  (page-ref "X") (property k v) (between …) …\n  ]` pre-filled from the post-1c supported
  heads. Because it's datalog, `Macro.tsx` flips to `runAdvancedQuery` automatically.
- Optionally a reverse "back to visual builder" only when the datalog is still
  builder-representable (nice-to-have; can defer).
- Confirm `advInfo` (ran/ignored) renders for the skeleton so users see which lines took.

## Verification
- `npm test` (both configs) + `cargo test -p tine-core` for the engine.
- Visual: screenshot-harness a `{{query}}` with an aggregation control + a group-by, per the
  visual-verification rule (mock must return `properties`).
- Round-trip is the core invariant: `toDsl(parseQuery(dsl)) === dsl` for every new clause.

## Docs to update when shipping
- `docs/FEATURES.md` (query section), `CHANGELOG.md [Unreleased]`, remove the item(s) from
  `docs/BACKLOG.md` Next, and the comparison page (`website/compare.html`) "math on queries"
  row this closes.
