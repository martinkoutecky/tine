# Query table footer

A live query table displays the query engine's complete statistics, including
matches outside the displayed sample. `Macro.tsx` supplies statistics and the
executed view to `SheetTable`; the current view and `apply` callback own edits.
`querySummary` is the only formatter of these backend statistics. Missing
statistics or a missing aggregate entry displays no total; backend markers
retain their unavailable explanation.

Property columns offer None, Count, Sum, and Average. Builtin and formula
columns offer no query aggregate. Changing or removing a column's first entry
edits it in place through the existing query display writer; other entries,
including repeated entries and the whole-result count, keep their order.
Children-backed sheets retain their existing sheet aggregate vocabulary.

Footer work is bounded by the supplied columns and statistics, with no graph
read or query execution. Editing uses the existing guarded query display
operation and its undo unit. Unit cost: no persisted record change; the
existing query block property is updated, with no new files or transport records.

Static export totals summarize the rows that are exported, as on master.
They can differ from live totals. The published-pages boundary excludes
unpublished rows before exporting; unrestricted query statistics never cross
that boundary. Export retains the existing sheet vocabulary and formula totals.

I-12 exemplar: `src/components/SheetTable.tsx` formats supplied statistics and
routes choices through `queryDisplay.apply`. Render outcomes are covered by
`SheetTable.queryFooter.test.tsx` and `staticExport.test.tsx`; the wiring and
export boundary are guarded by `src/sheet/ogBSheetGuards.test.ts`.
