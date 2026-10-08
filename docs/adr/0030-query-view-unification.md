# 0030. Query view unification

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

Tine supports two ways to enrich a query block: the query DSL and visual builder
choose which blocks belong to the result set, while `tine.view:: table` and
`tine.view:: board` choose a sheet face for those results. Before this decision,
those surfaces were mutually exclusive in the UI: once a query block had a sheet
face, the query header and builder disappeared, so the user could edit the view
but not the membership.

OG Logseq also has `:table-view? true` in the query options map. That table is a
read-only presentation toggle inside the query DSL. Tine's sheet table is editable,
round-trips as a block property, and shares the same table/board infrastructure
as child-sourced sheets.

## Decision

We will keep the query header and visual builder visible on query blocks that have
sheet faces. The query block owns membership through the query DSL and builder;
`tine.view::` owns presentation. The List/Table/Board switcher writes ordinary
block properties: Table writes `tine.view:: table`, Board writes
`tine.view:: board` and defaults `tine.group-by:: state` only when the block has
no grouping, and List removes `tine.view::`.

We deliberately supersede OG's interactive query-table toggle in Tine's header
with the sheet table switcher. Existing `:table-view? true` queries keep rendering
the legacy read-only table when they have no `tine.view`, but the header control
no longer writes session state or edits the query options map.

## Consequences

Query membership and query presentation can now be edited side by side on the
same block, including query boards and query tables. View choice becomes durable,
syncable plain text, and it round-trips through OG as harmless `tine.*`
properties.

This is a deliberate OG-parity divergence in the control surface: OG's query
table option is preserved for rendering old graphs, but Tine's active Table view
means the editable sheet table. The switcher must stay property-driven, and any
future query face must respect the same membership-versus-presentation boundary.
