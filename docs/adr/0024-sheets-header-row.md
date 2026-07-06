# 0024. Sheets positional-grid header row is explicit and opt-in

- **Status:** Accepted (decided autonomously during the Sheets build mandate — Martin should review)
- **Date:** 2026-07-06

## Context

Spec §13.4 left open whether a positional grid's first row should be treated as
a header. Options: (a) always treat row 1 as header (spreadsheet default), (b)
auto-detect (heuristics on formatting), (c) explicit opt-in via the reserved
`tine.header::` property (§3.1). TreeSheets has no header concept; free-form
grids (the positional face's main use) often have none; field-keyed tables get
real headers from field names and never need this flag.

## Decision

We will make the header row **explicit and opt-in**: `tine.header:: true` on
the grid's parent block marks the grid's **first row** as a header. No
auto-detection, no default header. When set, the first row renders with header
styling (bold, distinct background, sticky within the grid's scroll container)
and is excluded from data operations that iterate rows (sort, aggregates,
fill-down ranges clamp below it). The flag is meaningful for the positional
face only; field-keyed tables and boards ignore it (their headers derive from
fields / group values).

## Consequences

- Zero magic: a plain 2-deep outline renders as a plain grid; OG renders
  `tine.header:: true` as one more harmless property. Misdetection is
  impossible.
- The cost is one extra step for users who want a header (context menu / a
  "header row" toggle — ship the toggle in the same phase that ships sorting,
  which is where the header distinction starts to matter).
- Aggregates/sort/fill must all consult one shared `hasHeader(config)` accessor
  so the exclusion rule can't drift between operations.
