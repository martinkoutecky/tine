# Export definitions

An export definition is an ordinary query block (`{{query …}}` or
`{{tine-query …}}`) that carries a destination property on the block:
`export:: <folder>` or `tine.share:: <login>/<gist_id>`. It can live on any
page. `tine_core::export_definition` is the one answer to "is this block a
definition" (`is_definition`, `DESTINATION_KEYS`); the OG-readable query
listing every definition is `definitions_query()`,
`(or (property export) (property tine.share))`.

## The "this page" selector

A definition that shares one page selects it with the TQL page row

```
{{tine-query @page and name = 'Project'}}
```

This is the printer's own spelling, so a definition the dialog writes stays
byte-identical when the query builder reopens and saves it; the typed short form
`@page name = 'Project'` is the same query. It is an ordinary term: the builder
may combine it (`@page and name = 'Project' or name = 'Notes'`). `name`
compares the page's own name, case-insensitively (the page-identity fold), and
never an alias.

- **Page-anchored.** The page itself is the row, so a page with no blocks is
  selected and exported.
- **No `{{query}}` spelling.** OG's DSL has no page-row name form: `(page "x")`
  matches the page's blocks (OG `query_dsl.cljs` `build-page`), and its meaning
  is unchanged. A Tine-only head inside `{{query}}` would mean something else in
  OG Logseq (D-18), so the builder saves a selector as `{{tine-query}}`.

## Renames

A query macro creates no page reference (OG `graph_parser/block.cljs`), so the
reference rewrite of a rename never sees a quoted name. Renaming a page also
rewrites, **inside definition blocks only**, every quoted page name of an active
TQL `name` comparison (`=`, `!=`, `in`, `not in`, and a block row's
`page.name`) that names a renamed page, namespace descendants included. A new
page created under the old name is therefore not what the definition shares.

- Found by the TQL parser (`tine_core::query::tql::page_name_literals`) inside
  the macro extents of blocks the outline parser delimits, Markdown and Org; the
  rest of the file is byte-identical.
- Part of `tine_core::refs::rename_rewrite`, the one rename rewrite the planner,
  the store transaction and the page host share, so the rename's existing guarded
  write path writes it. The planner adds to its candidates the pages holding a
  definition whose selector names a renamed page: `definitions_query()` over
  the planning view, then `selected_names` over the returned blocks, in memory,
  so a definition page the rename does not change is not read.
- Not rewritten, deliberately: an ordinary (non-definition) query block, as in
  OG; any `{{query}}` text; `like`, `namespace` and property comparisons; a
  selector naming an alias; a macro whose new argument would no longer be one
  macro (a new name containing `}`), which stays as it was.
- A view whose definitions exceed one bounded query answer (20 000 rows or
  32 MiB) refuses the rename before writing (scenario: malformed or generated input, where a later page
  with the old name would otherwise be exported).

Unit cost: no new persisted record, file or transport record; a rename writes
the definition pages whose selectors change, through the existing rename
transaction. Planning adds one in-memory definitions query, O(pages + blocks),
memoized per graph generation, plus one TQL parse per returned definition
block; the rewrite adds one outline and TQL parse per candidate page holding a
`{{tine-query` macro.

Tests: `crates/tine-core/src/export_definition.rs` (spelling, round trip,
Markdown, Org, CRLF, scope) and
`crates/tine-graph-features/tests/share_selector.rs` (empty page exported; a
rename through the page host carries definitions, leaves an ordinary query and
an alias, and a new page under the old name is not selected).
