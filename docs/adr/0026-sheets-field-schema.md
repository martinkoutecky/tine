# 0026 — Sheets field schema (`tine.fields::`)

Status: accepted (Jul 7 2026, Phase 6a; settled autonomously per the Sheets
build mandate — listed under "Decisions Martin should review")

## Context

Sheets v1 is zero-schema: table columns are whatever field ids are *observed*
across the rows, in first-seen order. That means stray `key::` typos become
columns, column order churns as data changes, every cell is untyped text, and
add-row can't know what a row should look like. Spec §3.3 sketches a
`tine.fields::` schema as the v2 "supertag" layer and explicitly prefers
per-tag schemas over Obsidian's vault-global `types.json`.

## Decision

**Grammar.** `tine.fields:: name=type;name=type;…` — same `;`-separated
`k=v` shape as `tine.col-widths` / `tine.col-aggregates` (one config-grammar
family, parsed in `src/sheet/config.ts` beside them). Types:

- `text | number | date | datetime | checkbox | list | enum:a,b,c | ref`
  for property columns (`prop:<name>`).
- A name equal to a built-in field id (`state`, `priority`, `scheduled`,
  `deadline`, `tags`, `page`) declares the *built-in* column's position in
  the schema order; its type token must repeat the name (`state=state`).
  Built-ins keep their existing renderers/editors — the schema only orders
  and includes them.

Enum values ride comma-separated inside the token. All names and values obey
the §3.1 scalar rule (no `[[ ]] (( )) {{ }} #` or newlines); malformed
entries are *skipped*, never fatal — an unparseable schema degrades to v1
zero-schema behavior. **No default values in the grammar** (a `name=type:def`
extension can be added later without breaking existing files).

**Where the schema lives.** Two homes, one grammar, one parser:

1. Any view-owning block (children/query tables) — `tine.fields::` in its own
   properties, like every other `tine.*` config.
2. A tag page — `tine.fields::` in the tag page's page properties; the tag
   table on that page (and Phase-6c tag boards) read it.

Resolution for tag tables: the view block's own `tine.fields::` (when the
view is a query block) **wins over** the tag page's. The synthetic
`tag-page:<name>` table has no owning block, so it reads the page property
directly.

**Declared vs observed columns.** Declared fields render first, in schema
order; observed strays follow in first-seen order, visually marked (italic
header) with a header-menu "declare" affordance. Strays are **never hidden** —
data stays visible (the spec's honesty rule).

**Typed cell semantics** (canonical stored forms — write side; render side is
tolerant of foreign values and falls back to plain text):

- `checkbox` — stores `true` / `false` (lowercase, OG's boolean prop form).
- `number` — plain decimal text; non-numeric input rejected at the editor.
- `date` / `datetime` — ISO `yyyy-mm-dd` / `yyyy-mm-dd HH:MM` on write;
  renderer also accepts what it can parse and shows a date badge.
- `list` — comma-separated scalars, rendered as chips.
- `enum:a,b,c` — one of the declared values, rendered as a chip; editor
  offers exactly the declared values + clear.
- `ref` — a `[[Page Name]]` value rendered as a page-ref chip. (Data props
  may contain refs; the scalar rule constrains `tine.*` *config* values
  only.)

**Add-row seeding** stays minimal: children table → new empty child row,
editor opens on the title cell; tag/query table → today's-journal block
seeded with the tag ref only. **No empty `key::` lines are written** (OG
renders empty property lines as junk); declared columns simply show empty
typed cells inviting a click.

## Consequences

- Stable column order and typed cells arrive without any new file syntax
  beyond one more `tine.*` scalar property — plain-Logseq round-trip is
  untouched (OG shows the schema as an inert property line).
- Per-tag schemas beat a global types file: two tags can give the same
  property key different types; moving a graph moves its schemas.
- Malformed-tolerant parsing means a hand-edited schema can silently drop an
  entry; the header menu (6b) is the guided editing path.
- Built-ins-by-name keeps one field-id namespace; a user property literally
  named `state` cannot be declared (acceptable: OG semantics already make
  that key confusing).
