# 0028 — Sheets formula DSL (Phase 7)

Status: accepted (Jul 7 2026, Phase 7a; settled autonomously per the Sheets
build mandate — listed under "Decisions Martin should review")

## Context

Phase 6 gave tables typed, schema'd columns; the missing database leg is
computed values ("days until due", "price × qty", "overdue?") usable as
columns, filters, and group-by axes. Spec §6 already chose the model — the
Obsidian Bases formula DSL (JS-flavored typed expressions) — and rejected
TreeSheets operator cells and A1 references. This ADR makes the subset,
types, encoding, and evaluation contract normative.

## Decision

**Grammar (v1 subset).** Literals: numbers, single/double-quoted strings,
`true`/`false`, `null`, duration strings (`'7d'`, `"2w"` — a string literal
matching `^\d+[smhdwMy]$` used in date arithmetic). Field references: bare
identifier = property name (`price`); `state`, `priority`, `scheduled`,
`deadline`, `tags`, `page` = builtins; `formula.<name>` = another formula.
Operators: `+ - * / %`, comparisons `< <= > >= == !=`, logical `&& || !`,
unary minus, parens. Method chaining on typed values and free functions.
NO user-defined functions, assignment, indexing, or lambdas. A real
lexer + Pratt parser in `src/sheet/formula/` — the ONE parser for this
grammar, O(n), AST cached per raw-expression string.

**Types + errors.** `text | number | boolean | date | duration | list |
null`, plus `error`. Coercion is minimal and explicit: no implicit
string→number; `date ± duration` works; `+` on text concatenates only if
both sides are text; comparisons are type-strict. Errors are VALUES that
propagate (like Bases): an erroring cell renders an ⚠ chip with the message
on hover — never thrown to the UI, never written to disk.

**Stdlib (the 20%).** `if(c,a,b)`, `isEmpty(x)`; text
`.contains/.lower/.trim/.replace/.length`; number
`.round/.floor/.ceil/.abs/.toFixed(n)`; date `now()`, `today()`,
`.format(fmt)`, `.year/.month/.day`, `.relative()`; list
`.length/.join(sep)/.contains(x)`. Unknown function/method = an error value
naming it. Everything else deferred.

**Encoding.** One formula per property line: `tine.formula.<name>:: expr`,
name = `[a-z0-9-]+`. Never packed into `=;` lists (expressions contain
`=`/`;`). Two scalar-safety escapes, applied by the encoder and reversed by
the decoder: nested opening parens serialize as `( (` (OG would parse `((`
as a block ref), and `#` inside string literals is escaped (`\#`). Formula
properties live on the view-owning block AND (like `tine.fields::`) on a
tag page's page properties; the view block wins.

**`formula:<name>` is a pseudo-field**: a read-only table column (typed
rendering by result type), a board group-by axis, and — same evaluator — an
optional `tine.filter:: expr` on tables/boards (ships in 7c). Values are
derived, NEVER stored.

**Evaluation contract.** Pure evaluator over a per-row context that reads
ONLY through the existing facet source; results memoized per (block,
formula, data revision); `now()`/`today()` are per-render-pass constants
(no live ticking). Formulas referencing formulas form a DAG; a cycle is
detected at bind time and every member evaluates to a cycle error. Perf
budget: O(rows × formulas) per pass, log-warn above ~10k evaluations
(mirrors the v1 cell-count warn).

## Consequences

- One expression engine serves columns, filters, and group-by — no second
  grammar later.
- Bases compatibility of spirit (not syntax-exact) keeps the learnable-DSL
  story from the competitor study without importing its file format.
- Error-as-value means a half-typed formula degrades a column to ⚠ chips
  instead of breaking the table; the validating editor (7c) is the
  guardrail that makes the DSL usable.
- Duration strings keep date math scalar-safe inside a property line.
- The property-line escapes are the one place the encoding is cleverer
  than OG-visible text; both are round-trip-tested in both directions.
