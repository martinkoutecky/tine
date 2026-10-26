# Export definitions

An export definition is an ordinary query block (`{{query …}}` or
`{{tine-query …}}`) that carries a destination property on the block:
`export:: <folder>` or `tine.share:: <login>/<gist_id>`. It can live on any
page. `tine_core::export_definition` is the one answer to "is this block a
definition" (`is_definition`, `DESTINATION_KEYS`); the OG-readable query
listing every definition is `definitions_query()`,
`(or (property export) (property tine.share))`.

## The exports page

A page whose page properties carry `tine/exports:: true` (Org
`#+tine/exports: true`) is the exports page: Tine creates it on first use with
`public:: false` too, so OG Logseq's publishing skips it, and finds it by the
marker, so a rename keeps it working. It holds the definitions the share dialog
writes and an overview query, `definitions_query()`.

A quick share marks its block `tine.share:: pending/<record id>` before the
upload, writes `<login>/<gist_id>` in its place once the gist exists, and removes
it when the share fails, is deleted or forgotten (`docs/contracts/quick-share.md`,
"In the graph"). A pending marker is a definition too: the overview lists it.

## Repeating exports

Folder definitions offer **Re-export**, opening the existing review dialog with
the recorded leaf under `<graph>/published-queries/` and Replace selected.
Confirmation uses the reviewed fingerprint, sheet evaluator and guarded publisher;
the previous leaf is retained in recovery. No folder Delete action is offered.

**Update all** on the marked Exports page prepares both destination kinds before
one confirmation. Folder previews list whole pages and left-out reasons; block
queries require the whole-page acknowledgement. Folder membership diffs come from
the prior leaf's `app/snapshot.json` (excluding the synthetic home), read through
a graph-confined capability with the existing 128 MiB export ceiling. Missing,
invalid, oversized or inaccessible snapshots show the count only. No new record is stored.
Quick shares retain their membership diff. Folder exports remain available when
quick share is disabled or disconnected. Updates run sequentially and show each
outcome, continuing after failures. Closing or switching graphs prevents further
updates; a write already started may finish. No definition or output format changes.

## What every export leaves out

`render::export_left_out` is the one rule set, applied by the query export, the
live site and the CLI (quick share uses the query export's plan):

- a page marked `public:: false` (privacy settings apply on every run);
- the exports page, even when a query selects it and even with every page public.

The query export's plan lists the pages it left out (`leftOut`, with the reason)
and says whether the export includes the query's own page (`selectsHostPage`).
A query whose every selected page is left out is refused.

## The query text

A query export's published home lists the exported pages and carries nothing of
the query (no macro, no parsed query, no answer record) unless the request sets
`includeQueryText` ("Include the query text", off by default). The choice is not
part of the reviewed fingerprint. A query that is part of an exported page's own
content is that page's content and ships with it.

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

Tests: `crates/tine-graph-features/tests/query_publication.rs` (left-out pages, exports page, query text) and `crates/tine-core/src/export_definition.rs` (spelling, round trip,
Markdown, Org, CRLF, scope) and
`crates/tine-graph-features/tests/share_selector.rs` (empty page exported; a
rename through the page host carries definitions, leaves an ordinary query and
an alias, and a new page under the old name is not selected).
