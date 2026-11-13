# Export definitions

In printed pages and static HTML, `{{namespace X}}` lists stored page names
whose page identity starts with the identity of `X` followed by `/`, at any depth. Each
identity appears once, using its active file owner's spelling; links retain
that spelling and sort by displayed-name byte order. Alias-only and
reference-only names are absent. Static HTML applies the export's page
admission filter to the list. Namespace lookup ranges the existing store
name tree; it does not copy the corpus or visit unrelated page names.

An export definition is an ordinary query block (`{{query …}}` or
`{{tine-query …}}`) that carries a destination property on the block:
`tine.export:: <folder>` or `tine.share:: <login>/<gist_id>`. It can live on any
page. `tine_core::export_definition` is the one answer to "is this block a
definition" (`is_definition`, `DESTINATION_KEYS`); the OG-readable query
listing every definition is `definitions_query()`,
`(or (property tine.export) (property tine.share))`.

## The exports page

A page whose page properties carry `tine.exports:: true` (Org
`#+tine.exports: true`) is the exports page: Tine creates it on first use with
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
updates; a write already started may finish.

## What every export leaves out

`render::export_left_out` is the one rule set, applied by the query export, the
live site and the CLI (quick share uses the query export's plan):

- a page marked `public:: false` (privacy settings apply on every run);
- the exports page, even when a query selects it and even with every page public.
- comment threads when **Include comments** is off (or CLI `--no-comments`):
  their roots and every reply are omitted; the body host remains. This is a
  block projection beside the page rules, before the private build graph.

Comments are included by default, shown in the viewer's margin and as labelled
annotations in static HTML/print, with author and date where known. Text and OPML
prefix comment text with `[Comment]`; Markdown/Org retain the source comment
properties so they round-trip, including with Remove properties selected.
Copy outline and the private clipboard retain their full source content.

Repeating definitions carry `tine.comments:: false` when excluded; absence means
include. Re-export and Update all read it. Dialogs save only an explicit changed
choice through ordinary guarded block edits. Update all shows mixed saved choices
with an indeterminate checkbox; changing it overrides the batch and prepares a new
review. The effective comments choice is part of the reviewed fingerprint.

Unit cost: an excluded Markdown definition adds 22 UTF-8 bytes plus indentation
(`tine.comments:: false` plus LF); Org adds 22 bytes plus indentation for
`:tine.comments: false` plus LF, and 19 bytes for a new property drawer when one
is needed (a destination already supplies one). ASCII fixture byte counts:
Markdown 1-block 48→72, 60-block 461→485 (+24 with two-space indentation);
Org 1-block 65→87, 60-block 478→500 (+22 with its existing drawer).
On both page sizes ordinary edits add 0 bytes, files, transport bytes, reads,
stats or IPC. Changing the choice edits the existing page through its normal
whole-file guarded save (one file); the uncompressed file transport payload
grows by the same 24/22 bytes. No new transport record, file, cache
record or per-edit work. Projection costs O(exported blocks), within the existing
export walk; default-inclusion publication performs no projection clone.

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

A query macro creates no page reference (OG `graph_parser/block.cljs`), but a
rename rewrites the page-name arguments of **every** query like links
(D-20261031-CMQ; what a rename rewrites is the module doc of
`crates/tine-core/src/refs_rename.rs`).
For a definition this means every quoted page name of an active TQL `name`
comparison (`=`, `!=`, `in`, `not in`, and a block row's `page.name`) that
names a renamed page, namespace descendants included. A new page created under
the old name is therefore not what the definition shares.

- Found by the TQL parser (`tine_core::query::tql::page_name_literals`) inside
  the macro extents of blocks the outline parser delimits, Markdown and Org; the
  rest of the file is byte-identical.
- Part of `tine_core::refs::rename_rewrite`, the one rename rewrite the planner,
  the store transaction and the page host share, so the rename's existing guarded
  write path writes it. The planner adds to its candidates every page holding a
  query macro whose argument spells a renamed name (an in-memory scan of the
  planning view's parsed blocks), so a page the rename does not change is read,
  not written.
- Not rewritten, deliberately: `like`, `namespace` and property comparisons; a
  selector naming an alias; a macro whose new argument would no longer be one
  macro (a new name containing `}`), which stays as it was.
- Code is literal. A macro inside an inline code span, a fence or an Org
  `=…=`/`~…~` span is no macro, in a block's text and in a property's value
  (the outline parser keeps a property line as one node, so
  `refs_rename::PropertyValueCode` asks the parser for the value read as
  text, the way the app shows it, once per value that holds a macro, however
  many it holds); the same macro outside code follows.
- A name in **double quotes** is not a name in TQL: as in SQLite, `"Old"` is
  an identifier, so `name = "Old"` is an invalid query (it shares nothing) and
  has no literal to follow. The selector's spelling is the single-quoted one
  above; the OG DSL's quoted `(page "Old")` is a string there and follows in
  its own spelling.

Unit cost: no new persisted record, file or transport record; a rename writes
the pages whose query arguments change, through the existing rename
transaction. Planning adds one in-memory scan: O(blocks) checks for `{{`, a
case fold of each block holding one, and one block parse only where that fold
contains a renamed name (or the name is escaped); the rewrite adds one outline
and query parse per candidate page holding a query macro, and one more read of
each property value that holds one (counted by `refs_rename::tests`).

Tests: `crates/tine-graph-features/tests/query_publication.rs` (left-out pages, exports page, query text) and `crates/tine-core/src/export_definition.rs` (spelling, round trip,
Markdown, Org, CRLF, scope) and
`crates/tine-graph-features/tests/share_selector.rs` (empty page exported; a
rename through the page host carries definitions and ordinary queries, leaves
an alias, and a new page under the old name is not selected).
