# Sheets Phase 7 — formula columns (the Bases DSL)

Status: **planned, not started** (drafted Jul 7 2026 while the Phase-6
adversarial review ran; execution starts after the review's P1s are fixed
and — per the mandate — continues only if Martin says keep pushing).
Regime unchanged: branch `sheets`, codex-first tight specs, orchestrator
verifies + fixes, §3.8 atomicity, md+org round-trip gates, screenshots +
real-app e2e per chunk.

## Why this phase

Spec §6 Tier 2. With schema'd typed columns (Phase 6) the missing database
leg is computed values: "days until due", "price × qty", "overdue?" as a
filterable, groupable column. The model was decided in the spec after the
competitor study: **the Obsidian Bases formula DSL** (JS-flavored typed
expressions, method chaining, duration-string date math), NOT TreeSheets'
operator cells and NOT A1 references (both explicitly rejected in §6).
Study with full function inventory: `subagent-tasks/notes/
brainstorm-obsidian-bases.md` (local, gitignored — restate needed parts in
task specs).

## Design decisions to settle FIRST (each → ADR or spec edit)

1. **Grammar subset (make normative):** literals (number, single/double
   string, boolean, null, duration strings `'7d'`), field references (bare
   identifier = property name; `state/priority/scheduled/deadline/tags/page`
   = builtins; `formula.<name>` = other formulas), binary ops
   (`+ - * / % < <= > >= == != && ||`), unary (`! -`), grouping parens,
   method chaining on typed values (`.toFixed(2)`, `.contains(x)`), free
   functions (`if(c,a,b)`, `isEmpty(x)`, `now()`, `today()`). NO user
   functions, NO assignment, NO indexing/lambdas in v1.
2. **Type system + errors:** text/number/boolean/date/duration/list/null;
   Bases-style minimal coercion (string→number never implicit; date ±
   duration; comparisons type-strict, mismatch = error value). An erroring
   cell renders an ⚠ chip with the message on hover — errors are VALUES
   (propagate through expressions), never thrown, never written to disk.
3. **Stdlib (the 20%):** `if`, `isEmpty`; string `contains/lower/trim/
   replace/length`; number `round/floor/ceil/abs/toFixed`; date
   `now/today/format/year/month/day/relative`; list `length/join/contains`;
   checkbox truthiness. Everything else deferred; unknown call = error
   value naming the function.
4. **Encoding (spec §6 already decided — make the escapes normative):** one
   formula per property line, `tine.formula.<name>:: expr`; name =
   `[a-z0-9-]+`; scalar-safety: encoder writes `( (` for nested opens
   (avoids OG's `((` block-ref parse) and escapes `#` inside string
   literals; decoder reverses both. Formula properties live on the
   view-owning block AND (like `tine.fields::`) on a tag page; view wins.
5. **Semantics of `formula:<name>` as a pseudo-field:** usable as a table
   column, a board group-by axis, and (new, small) a WHERE-filter on
   tables/boards (`tine.filter:: <expr>`? — decide whether filter ships in
   7c or is deferred; leaning ship, it's the same evaluator). Derived,
   never stored; read-only cells (no editor).
6. **Evaluation + perf contract:** pure evaluator over a per-row context
   (facets + properties via the ONE facet source); results memoized per
   (block, formula, cache-gen) and recomputed when the store's data
   revision ticks; `now()`/`today()` are per-render-pass constants (no
   live ticking). Budget: parsing each formula once (AST cached by raw
   string), evaluation O(rows × formulas); log-warn above ~10k
   evaluations per pass (mirrors the v1 cell-count warn).
7. **Cycle handling:** formulas referencing formulas form a DAG; cycles
   detected at parse/bind time → all members render the cycle error.

## Sub-phases

### 7a — the expression engine (pure, no UI)
`src/sheet/formula/` : lexer → Pratt parser → typed evaluator, error
values, stdlib, duration parsing, encoder/decoder for the property line
escapes. ONE parser for this new grammar (a real lexer/parser, no regex
soup), O(n). Exhaustive unit tests incl. every stdlib function, coercion
matrix, error propagation, cycle detection, encode/decode round-trip
(property-line escapes), fuzz-ish random-expression round-trip
(parse→print→parse fixed point).

### 7b — computed columns
`tine.formula.*` parsed beside the other config (through facets, one
recognizer); `formula:<name>` FieldId kind; SheetTable renders computed
columns (typed rendering from Phase 6 reused by result type; ⚠ error
chips); sort + footer aggregates work over computed values; columns are
read-only; schema menu lists formulas (rename/delete = property edits).
md+org round-trip fixtures.

### 7c — formulas as axes + the validating editor
Board `tine.group-by:: formula.<name>`; optional `tine.filter::` on
tables/boards (per decision 5); the formula editor popup (reuse the
action-menu/popup machinery): field-name + function autocomplete, live
parse errors inline, save = one property write. e2e: edit a formula in
the real app, see the column update, value never hits disk.

### 7d — docs + samples + polish
FEATURES/README/CHANGELOG sync; onboarding template + website demo regen;
sample pages (tine-test + org-graph) get a formula table; screenshots;
perf spot-check on the 2000-block bench page.

## Verification gates (unchanged)
`npm test` both configs + `npx tsc` + `cargo test -p tine-core`; round-trip
fixtures for every new property writer; screenshots; extend
`scripts/e2e-sheets.mjs`; progress doc + commit + push per sub-phase.

## Parked
Martin's v1 UX nits (still uncaptured — batch pass when he dumps the list).
Phase 6 leftovers (progress doc). Custom `values`-expression footer
summaries. Merged cells, split view, canvas face (v3).
