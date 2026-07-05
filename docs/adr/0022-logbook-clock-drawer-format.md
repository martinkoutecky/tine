# 0022. Logbook CLOCK Drawer Format

- **Status:** Accepted
- **Date:** 2026-07-05

## Context

Tine now writes Logseq time-tracking data into real user graphs. The bytes must
round-trip with OG Logseq because the same graph can be edited by Tine, OG, mobile,
and sync tools. The risky details are small but load-bearing: the `:LOGBOOK:`
drawer placement, English `E` weekday abbreviation, default seconds mode, the two
spaces after `=>`, and avoiding duplicate clock-ins on repeated saves.

The frontend needs the same logic for live marker transitions and elapsed badges,
but adding a TypeScript drawer parser would create a second source of truth for the
same drawer format.

## Decision

We will keep CLOCK drawer parsing and writing in `tine-core/src/logbook.rs`. The
frontend wasm wrapper includes that same Rust source file and exposes only small
helpers for marker transitions and summary rows.

Clock summaries sum the stored `=> H:MM[:SS]` spans instead of recomputing from
timestamps. Clock writes use local time formatted as `yyyy-MM-dd E HH:mm[:ss]`
with a fixed English weekday table (`Sun` through `Sat`), and seconds support
defaults to on unless `:logbook/settings :with-second-support?` is explicitly
false.

## Consequences

Tine writes the same CLOCK line shape OG expects and can safely display summaries
for OG-written drawers without rewriting them. The wasm build must be regenerated
whenever `logbook.rs` changes so the frontend helper stays in sync.

The Rust module intentionally performs a narrow drawer-aware line scan rather than
extending lsdoc's opaque `Drawer` AST. If Tine later exposes logbook drawers as a
visible editable table, that UI must still route through this module or replace it
with one shared structured parser.
