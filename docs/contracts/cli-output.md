# Command-line output contract

What `tine status`, `tine page show`, `tine journal`, `tine block show`, `tine refs`,
`tine search`, `tine query` and `tine pages` print, and what each JSON field means.
Source of truth for the payloads is `src-tauri/src/owner/verbs.rs` (and the two
`status` producers in `lease.rs` and `serve.rs`); the text rendering is
`src-tauri/src/owner/terminal.rs`. `src-tauri/src/owner/output_tests/mod.rs` reads this
file and compares it with both, so a field added to a payload without a row here, or a
row here that no payload carries, fails the build. Protocol, framing and handover are
in `tine-agents/specs/campaigns/2026-10-cli/PROTOCOL.md`; this file owns the *content* of
`ok` replies and of errors.

## One payload, two renderings

- The owner's reply to a verb is `{"id":…,"ok":<payload>}` or `{"id":…,"error":<body>}`.
  `tine --json` prints `<payload>` as is, on one line of stdout. Without `--json`
  the same payload is rendered as text. There is no second result type: the text is
  derived from the payload, and it may be shorter than the payload but must never
  read as more complete than the payload says it is.
- **Disclosure fields** are the payload fields whose value tells the reader that the
  answer is partial, approximate, refused or reinterpreted. Every disclosure field
  that is set must show up in the text output. They are marked `yes` in the
  *disclosure* column below, and `output_tests` renders each one set and checks the
  text says it (by the value it carries, not by exact wording).
- Every payload has `"schema":"tine.cli/1"` and a `kind`. A reader ignores fields
  it does not know (PROTOCOL §2.4); a field's meaning changes only with a new schema.
- Where a payload embeds an existing DTO the Rust type is named and its fields are
  not copied here; the rows beneath *Read by the text output* list the DTO fields the
  text rendering depends on, and the test checks they exist in a real payload.

Column *presence*: `always` (present, never null), `nullable` (always present, may be
`null`), `optional` (omitted when it does not apply).

### Spelling: `rev` and `revision`

Two different things, two names, used the same way in JSON and text:

- `rev` is the **content revision**: a hash of the page file's bytes, the string the app's
  guarded save compares (`FileRev`). Spec A4's `--rev` is this one. The text output prints it
  as `rev`.
- `revision` is the **host version**: `{incarnation, version}` of the page *as the page
  host holds it*, informational and scoped to one host incarnation. `null` for a page the
  host does not hold. It is never a save guard and the text output does not print it.

## Verbs

### `owner.status`

`tine status`. Answered without admission, so it also answers while the owner is busy or
stopping. Produced by the app's lease (`owner: "app"`) and by `tine serve` (`owner: "serve"`).
Kind `status`.

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"status"` |
| `owner` | string | always | no | `"app"` or `"serve"`: which process owns the graph |
| `graph` | string | always | no | the canonical graph root |
| `pid` | integer | always | no | the owner's process id |
| `version` | string | always | no | the owner's Tine version |
| `state` | string | always | no | `serving`; `busy` (stopped admitting, not yet stopping); or the stop's progress text (`stopping`, or `retrying: <reason>` while a stop waits to write a crash draft). Treat any other value as progress text |

### `page.show` / `journal`

`tine page show NAME`. The page as the page host sees it: a held page's buffer (unsaved text
included), else its file. Kind `page`. Errors: `not-found`, `ambiguous` (an alias of several
pages, `detail.candidates`), `too-large` (over the 64 MiB parse bound), `busy`.

`tine journal [DATE]` answers the same payload, kind `journal`. `DATE` is `YYYY-MM-DD`,
`YYYY_MM_DD`, `today`, or a title in the graph's journal format; absent means today. Errors
add `bad-args` for an unreadable date, and `not-found` when that day has no file.

The fields of both:

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"page"` or `"journal"` |
| `id` | string | always | no | the page's graph-relative file path, e.g. `pages/a.md`; the exact disk spelling |
| `rev` | string | always | no | content revision of exactly `text` (see above) |
| `revision` | object | nullable | no | host version; `null` when the host does not hold the page |
| `revision.incarnation` | integer | optional | no | the host incarnation the version belongs to |
| `revision.version` | integer | optional | no | the page's version within that incarnation |
| `read_only` | string | nullable | yes | `null` for an editable page; else the reason the file cannot round-trip safely, so Tine will not write it |
| `text` | string | always | no | the page's bytes as UTF-8 (exactly what `rev` hashes); at most 64 MiB. A page whose bytes are not valid UTF-8 produces no payload at all (see *Pages the parser refuses* below), so `text` is never a lossy reading |
| `page` | `tine_core::model::PageDto` | always | no | the parsed page: name, kind, title, pre-block, root blocks (`BlockDto`), format. Its own `rev` (nullable) and `read_only` (boolean) restate the two fields above |

Read by the text output: `page.name`, `page.pre_block`,
`page.blocks[].raw`, `page.blocks[].children`, `page.blocks[].properties`.

**Pages the parser refuses.** `page show` and `journal` parse the page's bytes before they build
the payload, and the store's parse validation refuses bytes that are not valid UTF-8, an outline
nested deeper than 128 levels, an Org headline nested deeper than 128 levels, and (as
`too-large`) bytes over 64 MiB. A refusal other than `too-large` has no error code of its own
today: the reply is `internal` with the diagnostic id `cli-store-failed` and the message
`cli-store-failed: Undecodable` (exit 1), and no `text` is returned. A journal file containing
`- \xff` is the pinned example. Naming a page that is not in the graph's page index is
`not-found`, not this error.

### `block.show`

`tine block show UUID`. A block and its descendants by its persisted `id::` UUID (runtime ids are
not accepted). Kind `block`. Errors: `bad-args` (not a UUID), `not-found`, `too-large`.

The preview is **bounded**: at most 2,000 nodes (the block itself counts) and a byte cap of
32 MiB minus 4 KiB, whichever is reached first. The request's optional `max_nodes` lowers the
node bound (clamped to 1..=2000); `tine block show` does not send it. `truncated` counts the
nodes left out; with a budget too small for even the root the block list is empty and
`truncated` is the whole subtree.

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"block"` |
| `block` | `tine_core::model::BlockPreview` | always | no | the subtree and its truncation count |
| `block.group` | `tine_core::model::RefGroup` | always | no | the source page name and kind, and the subtree as `blocks` (one root `BlockDto` with its `children`) |
| `block.truncated` | integer | always | yes | nodes omitted by the 2,000-node or byte bound; `0` = the preview is complete. The text output says how many were omitted |

Read by the text output: `block.group.page`, `block.group.blocks[].raw`,
`block.group.blocks[].children`, `block.group.blocks[].properties`.

### `refs`

`tine refs TARGET [--block]`. Linked references to a page, or (`--block`, `TARGET` a persisted
`id::` UUID) to a block. Kind `refs`. Errors: `bad-args` (`--block` with a non-UUID), `too-large`
(the reference set is over the result bound: 20,000 rows or 32 MiB; nothing partial is returned).

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"refs"` |
| `target` | string | always | no | the page name or block UUID asked about, as given |
| `block` | boolean | always | no | whether `target` is a block id |
| `groups` | array of `tine_core::model::RefGroup` | always | no | referring blocks grouped by source page. A group's `evidence` (bounded matched occurrences, with its own `truncated`) is highlight data only; the text output prints no evidence, so it is not a disclosure |

Read by the text output: `groups[].page`, `groups[].blocks[].raw`, `groups[].blocks[].children`,
`groups[].blocks[].properties`.

### `search`

`tine search TEXT [--pages N] [--blocks N]`. The graph search, run exactly as the app's search
box runs it. Kind `search`. The payload is `tine_core::query_plan::QueryExecution` with
`schema` and `kind` added. Limits: `pages` defaults to 20 and is clamped to 500, `blocks`
defaults to 50 and is clamped to 5,000; a request over the clamp is answered at the clamp, and
`has_more` is what tells a reader a limit cut the list. A limit of `0` switches that category
off: the engine does not search it, so its `has_more` is `false` and the hits hold none of it.

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"search"` |
| `hits` | array of `tine_core::query_plan::QueryHit` | always | no | page and block hits, tagged by `entity` (`page`, `block`), in the engine's order (each category's sort comes before its limit) |
| `diagnostics` | array of `QueryDiagnostic` | always | yes | query parsing or execution problems (`code`, `message`); the text output prints each |
| `explanation` | object | always | no | the declarative explanation tree of the search |
| `has_more` | object | always | no | whether limits cut each category |
| `has_more.pages` | boolean | always | yes | `true`: more page hits existed than the page limit returned. `false` means the list is complete **or** the category was switched off with `--pages 0` (not checked) |
| `has_more.blocks` | boolean | always | yes | `true`: more block hits existed than the block limit returned. `false` means complete **or** switched off with `--blocks 0` (not checked) |
| `cancelled` | boolean | always | yes | the search was cancelled and no partial results are returned. `tine search` names no cancellation lane, so nothing replaces its search and this is `false` today; the field is kept because it is part of the DTO and the text prints it when set |

Read by the text output: `hits[].entity`, `hits[].display_text`, `hits[].page`.

### `query`

`tine query TEXT [--tql]`. Runs the query text a `{{query …}}` macro holds (Logseq simple or
advanced form, decided by the one Rust discriminator) or, with `--tql`, a `{{tine-query …}}`
macro's text. Semantics are the query engine's and D-18's; nothing here decides what a form means.
Kind `query`. The payload is `tine_core::query::ir::QueryResult` with `schema`, `kind` and, when
present, `og_hint` added. Errors: `bad-args` when the query does not parse (`detail.diagnostics`
holds the parser's diagnostics; an advanced form is not refused here, it runs and reports), `too-large`.

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"query"` |
| `anchor` | string | always | no | `"block"` (rows are `groups`) or `"page"` (rows are `pages`) |
| `groups` | array of `RefGroup` | optional | no | block-anchored rows, grouped by page |
| `pages` | array of `tine_core::query::ir::PageRow` | optional | no | page-anchored rows: `path`, `name`, `kind`, `journal_day?`, `properties` |
| `diagnostics` | array of `Diagnostic` | always | yes | the query's enabled and disabled diagnostics (`message`, `kind`, `disabled`, `suggestions`, `span?`); a non-disabled one means the query returned nothing. The text output prints each |
| `report` | `QueryReport` | always | no | what an advanced form ran and ignored |
| `report.ran` | array of string | always | no | the clauses of an advanced form that ran |
| `report.ignored` | array of string | always | yes | the clauses of an advanced form that could not be run. Execution is all-or-nothing: any ignored clause means the whole query returned nothing |
| `report.supported` | boolean | always | yes | `false`: the advanced form was refused whole (rows are empty); `true` for every simple and TQL query |
| `total` | integer | always | no | matches counted (blocks for a block-anchored query, pages for a page-anchored one). With `exceeded` false it is the number of block roots (summed over `groups`) or pages returned; once `exceeded` is true it also counts matches the bounds dropped, so it can be larger than what the payload holds |
| `matched_total` | integer | optional | no | the exact complete stored match count, when the producer proved one |
| `statistics` | `QueryStatistics` | optional | no | aggregates the query asked for; not printed by the text output |
| `exceeded` | boolean | always | yes | the result hit the engine's row or byte bound and the list is incomplete. The text output says so |
| `og_hint` | `tine_core::query::wire_parse::OgQueryHint` | optional | yes | D-18's on-query cue, present only for a valid Logseq-syntax query that is left with no condition: `bare` lists the bare `(task)` / `(priority)` heads that added none (empty when the query is simply empty, e.g. `(and)`), `no_conditions` says nothing is left, and `rewrites` lists the explicit forms on offer. The text output names the bare heads and prints each rewrite's text |
| `og_hint.bare` | array of string | optional | no | `task`, `priority` |
| `og_hint.no_conditions` | boolean | optional | no | nothing is left, so the query shows nothing |
| `og_hint.rewrites` | array of object | optional | no | one per explicit form on offer |
| `og_hint.rewrites[].rewrite` | string | optional | no | `open_tasks`, `any_task` or `priorities` |
| `og_hint.rewrites[].query` | `Query` | optional | no | the rewrite as an IR query (the DTO's own field) |
| `og_hint.rewrites[].text` | string | optional | no | the rewrite printed as a Logseq query by the existing OG printer; `null` when the OG syntax cannot say it. Added by the CLI payload, not part of the DTO |

Read by the text output: `groups[].page`, `groups[].blocks[].raw`, `groups[].blocks[].children`,
`groups[].blocks[].properties` (block-anchored) and `pages[].name` (page-anchored).

### `pages`

`tine pages [--namespace NS]`. Existing pages (files), optionally those at or under a namespace.
Aliases and reference-only names are not listed. Kind `pages`.

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"pages"` |
| `namespace` | string | nullable | no | the filter as given, or `null` |
| `pages` | array of object | always | no | one object per page |
| `pages[].name` | string | always | no | the page's display name (decoded from its file claim) |
| `pages[].id` | string | always | no | the graph-relative file path |
| `pages[].journal` | boolean | always | no | whether it is a journal page |

Read by the text output: `pages[].name`.

### `owner.yield`

Not a read verb: `tine serve --stop`, and the app's handover. Its success payload is
`{"schema":"tine.cli/1","kind":"yield","yielded":true}`, sent while the owner still holds the
lock. It is produced by `serve.rs` and exercised by the handover tests, not by `output_tests`.

## The text outline

One block per line, indented two spaces per level:

```
# a
id pages/a.md rev 9e8b1eedbf3d8ef0
1 [author:: claude] one
  id:: 6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f
  1.1 under one
2 TODO two
```

- The address (`1`, `1.2`) is the block's outline position (PROTOCOL §10). `--if`-style content
  handles arrive with the write verbs.
- **Attribution sits beside the address** (spec A3: "each block line starts with its address and
  its `author::` when there is one"): `ADDRESS [author:: NAME] first line`. The `author::` property
  line is not repeated among the block's other lines. The JSON `raw` is unchanged and still holds
  it. Other properties are shown as lines under the block, as written.
- **Whether a block has an author is the projection's answer.** The text reads `properties`
  (`BlockDto.properties`, the DocBlock projection in `tine-core`, which treats a `key::` line inside a
  code fence as content) and does not scan `raw`. A literal `author:: NAME` line in a code fence is
  content: it is neither attribution nor hidden. A block with no `properties` entry for `author` has
  no attribution.
- Continuation and property lines are indented under the address.
- A block preview cut by its bound ends with a line naming how many nodes were omitted
  (`block.truncated`).

## Errors

A failed command prints one line to stderr and exits non-zero.

- Text: `tine: <code>: <message>`. A refusal puts its reason in the code so a script can match
  it: `tine: refused:<reason>: <message>` (spec A3). Ambiguity candidates and a query's
  diagnostics follow as indented lines (`  candidates: …`, `  diagnostic: …`).
- `--json`: one line, the envelope below, on stderr.

```json
{"schema":"tine.cli/1","error":{"code":"…","message":"…","detail":{"admitted":false}}}
```

| field | type | presence | meaning |
|---|---|---|---|
| `schema` | string | always | `"tine.cli/1"` |
| `error.code` | string | always | one of the codes below |
| `error.message` | string | always | prose for a person; not stable |
| `error.detail` | object | optional | owner replies always carry `detail.admitted`; a client-side failure (no reply) has no `detail` |
| `error.detail.admitted` | boolean | optional | whether the owner admitted the request (PROTOCOL §2.4); present on every owner reply |
| `error.detail.candidates` | array of string | optional | `ambiguous`: the page ids the name could mean |
| `error.detail.reason` | string | optional | `refused`: why (`draft-namespace`, `app-owner`, …); the text puts it in `refused:<reason>` |
| `error.detail.diagnostics` | array of `Diagnostic` | optional | `bad-args` from a query that does not parse |
| `error.detail.diag` | string | optional | `internal`: the diagnostic id (below) |

### Error codes and exit codes

One mapping serves a reply from an owner and a failure to reach or become one
(`terminal.rs::EXIT_CODES`; this table is pinned to it). `0` is success. An unlisted code (an
owner newer than this `tine`) exits `1`, like `internal`.

| code | exit | meaning |
|---|---|---|
| `not-found` | 3 | the page, block or journal day does not exist |
| `ambiguous` | 3 | a name matched several pages |
| `busy` | 4 | the owner is yielding or stopping; not admitted. Reads retry for 30 s first |
| `unknown-verb` | 2 | the owner does not know the verb (an older Tine) |
| `bad-args` | 2 | arguments invalid for the verb; also a query that does not parse |
| `frame-too-large` | 2 | the request line was over the cap |
| `malformed` | 2 | the request was not a request object |
| `too-large` | 2 | a read answer would exceed a result or page bound |
| `protocol` | 5 | the owner speaks another protocol version |
| `owner-mismatch` | 5 | the endpoint belongs to another graph (hash collision) |
| `owner-start` | 5 | `tine serve` did not come up, or the owner's endpoint does not answer, or the owner stopped answering a read |
| `owner-unsafe` | 5 | the owner namespace or endpoint path is not safe to use |
| `owner-lock` | 5 | the owner lock call itself failed (not contention) |
| `unsupported` | 5 | no owner endpoint on this platform yet (Windows, slice 1a) |
| `stale` | 6 | reserved for writes: the expected content changed |
| `refused` | 6 | the owner refused (`tine serve --stop` to an app-owned graph, or a draft-namespace mismatch on yield) |
| `draft-failed` | 7 | reserved for writes: admitted, draft not written, nothing changed |
| `unwritten` | 8 | reserved for writes: admitted, partial or pending |
| `superseded` | 8 | reserved for writes: applied, then discarded before publishing |
| `uncertain` | 8 | admitted, outcome unknown (the connection was lost after the request was sent; `tine serve --stop`) |
| `internal` | 1 | an owner bug or unnamed failure; the message leads with a diagnostic id |

Spec A3's "1 refused or failed" is superseded for refusals by PROTOCOL §2.3 (`refused` is 6);
exit 1 is `internal`, an unlisted code, and a `tine serve` failure that has no code of its own.
A usage error before any request (no graph found) exits 2.

`tine serve GRAPH` has the same mapping: its typed startup failures `owner-lock`, `owner-unsafe`
and `unsupported` exit 5 (the text on stderr starts `tine serve: <code>: …`); any other failure
exits 1; a clean stop, or losing the lock race to another owner, exits 0.

### Diagnostic ids of `internal` errors

An `internal` error message starts with one of these ids (`<id>: <detail>`), `detail.diag` repeats
it, and the owner logs the id on its stderr (`[tine] <id>`) with the detail in the debug log only.
The set is closed (`verbs.rs::Diag`) and this table is pinned to it.

| id | when |
|---|---|
| `cli-graph-load` | the graph's load failed, or the Store closed under a read |
| `cli-reply-encode` | a reply payload could not be encoded as JSON |
| `cli-read-failed` | a references, block preview or search read failed |
| `cli-query-failed` | a parsed query failed to run |
| `cli-store-failed` | the Store failed a page read in a way no code names |
