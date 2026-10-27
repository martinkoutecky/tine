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
32 MiB minus 4 KiB, whichever is reached first; each node is charged its estimated projected size
(id, text, properties, tags, breadcrumb and a fixed overhead), as the index charges it. The request's optional `max_nodes` lowers the
node bound (clamped to 1..=2000); `tine block show` does not send it. `truncated` counts the
nodes left out; with a budget too small for even the root the block list is empty and
`truncated` is the whole subtree.

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"block"` |
| `block` | `tine_core::model::BlockPreview` | always | no | the subtree and its truncation count |
| `block.group` | `tine_core::model::RefGroup` | always | no | the source page name and kind, and the subtree as `blocks` (one root `BlockDto` with its `children`); each block also carries its page-absolute `handle` when the page could be read (see *Text output*) |
| `block.truncated` | integer | always | yes | nodes omitted by the 2,000-node or byte bound; `0` = the preview is complete. The text output says how many were omitted |

Read by the text output: `block.group.page`, `block.group.blocks[].raw`,
`block.group.blocks[].children`, `block.group.blocks[].properties`, `block.group.blocks[].handle`.

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

### Writes: `block.add` / `block.edit` / `block.prop` / `block.move`

`tine add`, `tine edit`, `tine prop set|remove`, `tine move`. Every write goes through the graph's owner and
the page host's guarded save, the path the app's own edits use: the owner first saves any text typed in a window
holding the page, then reads the page, resolves the command's blocks against what it read, plans the new bytes,
checks they read back as the intended outline, and writes. If the page changed in between (someone typed), it
re-reads and re-resolves once; a second change is `stale`. Writes are never retried by the client.

- **Blocks** are named by a **handle**: `PAGE/POS~FP` as a read prints it (see "The text outline"), or a block's
  `id::` UUID (graph-wide). The position is used when its fingerprint still matches; otherwise the one block on the
  page with that fingerprint; otherwise `stale`. A bare `PAGE/POS` (no fingerprint) is accepted only with `--rev`.
- **`--rev REV`** (optional, every write): write only if the page is still at that rev; else `stale` with the
  current `detail.rev`.
- **`block.add`**: `--under|--after|--before BLOCK`, or `--page PAGE` (created when absent), or `--journal`
  (today) / `--journal=DATE` (created when absent); content `-` reads stdin. Content is a page fragment in the
  page's format (Markdown `- ` or Org outline). Every created block gets `author::` (from `--author`, else
  `TINE_AUTHOR`, else `agent`); edits never add one. An anchor without `id::` gets one in the same write. A
  `((uuid))` that names no block is refused `broken-ref`.
- **`block.edit`**: the text replaces the block's editor text, as typing it would: OG hidden built-ins
  (including SRS keys) and graph-configured hidden property lines are kept byte-exact; children are kept too.
  Raw text returned by reads may be submitted directly: an explicitly supplied primary property replaces
  the retained row with the same normalized key, so a read-then-edit never adds a second copy. This also
  permits an explicit new value, except that a supplied primary `id` differing from an existing accepted
  block `id` is refused (`reason: identity-change`) with both ids in the error. The in-scope threat is
  malformed agent input, such as submitting another block's raw text or mistyping its UUID.
  IDs compare after trimming, ignoring ASCII letter case, as block-handle resolution does. Same-id and
  omitted-id edits keep the identity; a block without an accepted id may receive one, even when its body
  contains a non-primary lookalike. A property-looking line inside a fence does not replace metadata.
  On a block whose own primary properties
  include `quote`, the four comment lines (`quote::`, `quote-prefix::`, `quote-suffix::`, `author::`) are also
  kept. On an ordinary block, `author::` and quote-context lines belong to the replaced editor text, as on OG.
  Text-only edits preserve the source position and order of hidden rows and LOGBOOK when that keeps
  property ownership and creates no more property groups than canonical placement. Otherwise the join
  restores canonical placement so OG can read the retained properties, including repairing earlier split groups.
  If the supplied text contains a parsed LOGBOOK drawer, that drawer replaces the retained one; fence lookalikes
  remain ordinary text. A marker change writes the logbook as the app does.
  Clients supplying LOGBOOK content must pass `--rev` from the read that supplied it. A clock-only change does
  not change an editor-text fingerprint: without that revision, an old full buffer can replace newer history.
  Plain visible-text edits retain the current history, including clock changes since the client's read.
  The CLI currently permits explicit history replacement without `--rev`; the revision requirement is a client
  concurrency obligation, not an enforced refusal.
- **`block.move`**: the block and its children go under (last child), after or before a target on the same page.
  A move to another page is refused `cross-page` in this version.
- **`--run RUN`** (optional, every write; or `TINE_RUN`): commit the write to the graph's git repository as
  part of a run (see *Runs*). Checked before the write: a graph with no usable repository is refused `git` and
  nothing changes. **`--author NAME`** (or `TINE_AUTHOR`, default `agent`) names the agent on created blocks'
  `author::` and on the run's commits.

Kind `written`. Write replies carry no page text.

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"written"` |
| `page` | string | always | no | the page id written (graph-relative path) |
| `name` | string | always | no | the page's name |
| `rev` | string | always | no | the page's rev after the write, from the written bytes; pass it as `--rev` to chain |
| `blocks` | array of string | always | no | the handles (`PAGE/POS~FP`) of the blocks created or changed, in the new text, in outline order; at most 200 |
| `more` | integer | always | yes | blocks created or changed beyond the 200 listed; the text output says how many |
| `git` | object | optional | yes | present only for a write with `--run`: its commit, or why it has none |
| `git.committed` | boolean | optional | yes | `true`: the write's bytes are committed. `false`: the file is saved but not committed (`git.error`, `git.message` say why: `refused:git` for a repository error, `refused:git-busy` when the branch kept moving) |
| `git.commit` | string | optional | no | the agent commit's id |
| `git.snapshot` | string | nullable | yes | the commit of the user's uncommitted edits to the file, made first under the user's git identity; `null` when `HEAD` already had the bytes the write started from |
| `git.index` | string | optional | yes | the committed file's index entry: `synced` (now matches the commit; changed under git's `index.lock` protocol, so a concurrent `git add` is never overwritten), `staged-kept` (the user had other content staged for the file; left staged, nothing of it lost), `busy` (the index was locked or conflicted; untouched, so `git status` may show the file changed until git refreshes) |

Read by the text output: `name`, `rev`, `blocks`, `more`, `git`.

Text: `written NAME rev REV`, then one handle per line, then `... and N more blocks` when `more` is non-zero,
then for a run write `committed COMMIT` (with `(your edits first: COMMIT)` when a snapshot was made) or
`warning: saved but not committed (ERROR): MESSAGE`, and a note when `git.index` is `staged-kept` or a warning
when it is `busy`.

### `block.delete`

`tine delete BLOCK [--rev REV]`. The block and its children, through the same write path. Kind `deleted`: the
`written` fields above (`blocks` is empty: the deleted blocks have no handle in the new text), plus:

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"deleted"` |
| `page` | string | always | no | as `written` |
| `name` | string | always | no | as `written` |
| `rev` | string | always | no | as `written` |
| `blocks` | array of string | always | no | as `written` |
| `more` | integer | always | yes | as `written` |
| `deleted` | string | always | no | the deleted block's handle, as it was before the deletion |
| `broken_refs` | integer | always | yes | `((uuid))` references elsewhere in the graph to an `id::` in the deleted subtree, not counting references inside the subtree itself (the app's delete notice counts the same way) |

Read by the text output: `deleted`, `broken_refs`, `name`, `rev`.

A run write's `git` object is as for `written`.

Text: `deleted HANDLE (breaks N references)`, then `NAME rev REV`, then the `git` line as for `written`.

### Runs: `run.begin` / `run.show` / `run.end` / `run.revert`

`tine run begin|show|end|revert` (CLI slice 3, D-20261019-CL1…CL4; desktop only). A run is the commits whose
message carries the trailer `Tine-Run: RUN`; nothing about a run is stored anywhere else. The graph must be inside
a non-bare git work tree, on a branch, with no merge, rebase or cherry-pick in progress; otherwise every run verb
and every `--run` write is refused `git` before it changes anything. A `--run` write while another git command
holds the index (`index.lock`) is refused `git-busy` before it changes anything. A message carries a trailer only
in its final trailer block, as `git interpret-trailers --parse` reads it; the id quoted in a message's prose does
not make a commit part of the run.

The owner makes one run write, `run end` or `run revert` at a time: a write's save and its commits are one step,
so commits follow the order of the saves.

Each `--run` write, after its guarded save succeeds, commits only the file it wrote, built from the exact bytes
(never from the working tree or the index), on top of the branch tip by compare-and-swap: first, if the file at
`HEAD` differs from the bytes the write started from, a snapshot of those bytes under the user's git identity
(trailer `Tine-Snapshot: RUN`); then the written bytes under `NAME <agent@tine.invalid>`. A graph file that git
ignores is not committed (`refused:git`). The file's index entry then follows the commit, unless the user had other
content staged for it (`git.index`).

Both raw-content writers (runs and autocommits) check the exact work-tree-relative path, including a nested
graph prefix, for `filter`, `ident` and `working-tree-encoding` attributes. An attribute set to a value or true
refuses a run write with `refused:git`, naming the path and attribute, before flushing or saving the page.
False and unspecified attributes allow exact bytes; Tine never executes external filters. The check repeats
before creating any snapshot or agent blob, so a conversion introduced after the save leaves the saved page
uncommitted without storing its raw content in Git.

Both writers revalidate the captured branch identity while holding `index.lock`, before moving a ref or
changing the index. A concurrent checkout refuses the commit explicitly; a run reply reports
`git.committed: false` and says the file was saved but its commit is missing. It never retries on the new
branch. A same-branch concurrent commit still uses compare-and-swap retries.

| Refusal | In-scope threat | Recovery |
| --- | --- | --- |
| `refused:git` naming a conversion attribute | repository content conversion, including git-crypt plaintext disclosure | use Git's own conversion workflow; no page, ref, index or raw object is changed by the refused run write |
| `git.committed: false` naming a changed branch | concurrent honest terminal checkout between repository open and index locking | inspect the saved page and active branch before starting another write; neither branch nor the new branch's index is changed by the refused commit |

- **`run.begin`**: kind `run`; `run` (the id, `r<hex seconds>-<8 hex>`), `branch`. Text: the id.
- **`run.show RUN`**: kind `run-commits`; `run`, `commits` (newest first: `commit`, `agent` (boolean; `false` =
  the user's snapshot), `author`, `subject`). Text: one line per commit.
- **`run.end RUN [-m MESSAGE]`**: kind `run-end`; `run`, `squashed` (the new commit, or `null`), `commits` (the
  run's agent commits), `kept_because` (nullable, disclosure: why nothing was squashed). It squashes only when the
  run's commits are contiguous at the branch tip, more than one, none reachable from another ref (a
  remote-tracking branch, another branch, a tag), and the tip has not moved; the squash has the tip's tree and the
  commit before the run as parent. Known limit: only local refs are checked. A push by URL that leaves no ref
  behind (`git push /path branch` with no remote configured), a copy of the repository made by a file sync
  (Syncthing) or a tracking ref deleted since cannot be seen, and the squash then rewrites commits another copy
  has. Text: `squashed N commits into COMMIT` or `kept N commits as they are: WHY`.
- **`run.revert RUN [--author NAME]`**: kind `run-revert`; `run`, `revert_run` (the revert's own run id),
  `pages` (one per page the run's agent commits touched: `page`, `reverted` (boolean), `rev`, `git` (as for
  `written`; the commit carries `Tine-Run: REVERT_RUN` and `Tine-Run-Revert: RUN`), `kept` (disclosure: blocks
  the revert left as the user has them, each `text` and `reason`: `changed-since` = the user changed it after the
  agent, `no-base` = the merge could not place it, `unclear` = an old block not brought back because the agent
  removed or rewrote it beyond pairing and the user has since changed or removed what the agent wrote; `text` is
  then the old block's first line), or `error` + `message` when that page was not reverted). The agent commits
  are undone newest first by a Concord three-way merge per page and one guarded write per page. Every page is
  read and merged before any is written: a page that cannot be read or merged refuses the whole revert with
  `refused:merge` and nothing changed. The pages are then saved one by one, not as one transaction: a page whose
  save fails is listed with its error and the pages saved before it stay reverted. The revert is itself a run, so
  `run revert REVERT_RUN` undoes its committed pages in one command; it is not a step of the app's Undo. Text:
  `reverted PAGE` / `unchanged PAGE` / `not reverted PAGE (ERROR): MESSAGE`, each kept block as
  `  kept your text (REASON): FIRST LINE` (`  not brought back (unclear whether you removed it) (unclear): FIRST
  LINE`), then `undo this revert: tine run revert REVERT_RUN`. `run revert` passes the same index-lock gate as a
  `--run` write.

Only commits within 10 minutes before the run's begin time and at most 20,000 first-parent commits back are
searched.

### `page.create`

`tine page create NAME` creates an empty page through the owner's guarded save,
with EditKind `create-page`. It creates no blocks, so there is no `author::` to
add. An existing page, alias or file refuses as `refused:page-exists`. Its payload
and text are the `block.edit` / `written` fields above (`blocks: []`, `more: 0`).
An absent page cannot match `--rev` (`stale`). `--run` uses the existing commits.

### `page.rename`

`tine page rename OLD NEW` calls the app's rename operation, including references,
namespaces, titles and the home page. A sole alias resolves to its canonical page.
A reference-only source or fileless namespace parent uses the same operation:
references and physical descendants move without creating a placeholder page.
A destination owned by a file or alias refuses as `refused:page-exists`; merging
requires confirmation in the app. Duplicate source claimants return `ambiguous`.
The window saves typed text first and receives the app's rename report to refresh
navigation and rewritten pages. A graph-wide rename cannot use the existing
single-file revision guard or run commits: `--rev` or `--run` (including `TINE_RUN`)
refuses as `refused:rename-options` before any change. An error from the app's I/O
adapter whose admission cannot be proved returns `uncertain` with
`detail.admitted: null`; inspect disk and any rollback diagnostic before retrying.
If page/referrer publication succeeded but the separate home-page config update
failed, it returns `uncertain` with `detail.admitted: true` and `detail.report`
containing the known applied page changes (`home_page: null`). The window receives
that report too. This error is a partial outcome, never a no-change refusal;
inspect the home-page configuration before retrying.

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"page-renamed"` |
| `old` | string | always | no | canonical file title, or trimmed source name for a fileless source |
| `name` | string | always | no | requested destination title, trimmed |
| `report` | `tine_graph_features::pages::RenameReport` | always | no | app operation report |
| `report.outcome` | string | always | no | `renamed` or `unchanged` |
| `report.touched` | array | always | no | old paths and whether they moved |
| `report.skipped_conflicted_referrers` | array of string | always | yes | referrers carrying VCS markers left unchanged; text prints each path |
| `report.home_page` | string | nullable | no | new home-page title when changed |

### `page.delete`

`tine page delete NAME --yes` (alias `--force`) uses the host's delete operation,
including graph trash and its checked-byte guard. Without confirmation it refuses
as `refused:confirmation`. A missing page is `not-found`, duplicate identities
are `ambiguous`, a stale `--rev` is `stale`, and read-only pages refuse. A pending
admitted deletion returns `unwritten`, `superseded` or `uncertain`, never success
before publication. `--run` commits the deletion.

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"page-deleted"` |
| `page` | string | always | no | deleted graph-relative file path |
| `name` | string | always | no | canonical page title |
| `broken_refs` | integer | always | yes | indexed linked page referrers plus block references broken by removing the page; text always prints the count |
| `git` | object | optional | no | run commit result, with the same disclosures as `written.git` |

### `ref` / `embed`

`tine ref BLOCK` prints `((uuid))`; `tine embed BLOCK` prints
`{{embed ((uuid))}}`. They ensure a durable lazy block id with the same property
editor as Copy block ref, through `save-block`. They insert no blocks and preserve
attribution. Handles, `--rev`, `--run` and failures follow block writes. Duplicate
ids within one page refuse as `stale` with candidate handles; ids claimed by
multiple pages refuse as `ambiguous`. The text is the reusable syntax alone, plus any run
commit disclosure; success means the id is written to disk.

| field | type | presence | disclosure | meaning |
|---|---|---|---|---|
| `schema` | string | always | no | `"tine.cli/1"` |
| `kind` | string | always | no | `"ref"` or `"embed"` |
| `page` | string | always | no | target's graph-relative file path |
| `name` | string | always | no | target's page title |
| `rev` | string | always | no | written page revision |
| `blocks` | array of string | always | no | target's full handle |
| `more` | integer | always | no | `0` (one target) |
| `uuid` | string | always | no | persisted block id |
| `text` | string | always | no | reusable reference or embed syntax |
| `git` | object | optional | no | run commit result, with the same disclosures as `written.git` |

## The text outline

One block per line, indented two spaces per level:

```
# a
id pages/a.md rev 9e8b1eedbf3d8ef0
1~eea65l [author:: claude] one
         id:: 6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f
  1.1~3as2k7 under one
2~hl6h22 TODO two
```

- The address of a page read is the block's **handle**, `POS~FP`: its outline position (`1`, `1.2`,
  PROTOCOL §10) and a 6-character fingerprint of its editor text (lowercase base32 of FNV-1a 64 over
  the shared built-in editor projection; OG hidden built-ins, including SRS keys,
  are left out, as are `quote::`, `quote-prefix::`, `quote-suffix::` and `author::` only when the block's own
  primary properties include `quote`). Ordinary author/context properties contribute to the fingerprint.
  Graph-configured hiding does not alter fingerprints; handles remain independent of that display preference.
  A write names a block as `PAGE/POS~FP` or by its `id::` UUID; the fingerprint
  resolves only when exactly one block on the page has it, else `stale`. The `--json` page carries the
  same string as each block's `handle`. Continuation lines align under the first line's text.
  A block preview (`block show`) prints the same handles: page-absolute positions (a block at `3.1`
  prints `3.1~FP`, its children `3.1.1~FP`). The preview's text, positions and fingerprints all come
  from one read of the page as the host holds it, so a handle it prints names the block it shows or
  is `stale`. The group's `page` is the name the graph's index resolved to the file it read, never
  a name the bytes declare since (a new `title::`), so the address a handle forms opens that same
  file. When the page cannot be read at that moment the preview comes from the graph's index
  and prints positions relative to the block, with no handles. Result groups (`refs`, `query`) print positions only.
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
| `error.detail.admitted` | boolean or null | optional | whether the owner admitted the request (PROTOCOL §2.4); present on every owner reply; `null` for `uncertain` (unknown) |
| `error.detail.candidates` | array of string | optional | `ambiguous`: the page ids the name could mean, or for a duplicated `id::` the names of (at most two of) the pages holding it; `stale`: the handles (`POS~FP`) of the blocks a handle could mean |
| `error.detail.page` | string or null | optional | a write's page id (`stale`, `refused` and the admitted write codes) |
| `error.detail.handle` | string | optional | `stale`: the handle that no longer resolves |
| `error.detail.rev` | string | optional | `stale` on `--rev`: the page's current rev |
| `error.detail.uuid` | string | optional | `refused` / `broken-ref`: the UUID no block has |
| `error.detail.reason` | string | optional | `refused`: why (below); the text puts it in `refused:<reason>` |
| `error.detail.diagnostics` | array of `Diagnostic` | optional | `bad-args` from a query that does not parse |
| `error.detail.diag` | string | optional | `internal`: the diagnostic id (below) |

### Error codes and exit codes

One mapping serves a reply from an owner and a failure to reach or become one
(`terminal.rs::EXIT_CODES`; this table is pinned to it). `0` is success. An unlisted code (an
owner newer than this `tine`) exits `1`, like `internal`.

| code | exit | meaning |
|---|---|---|
| `not-found` | 3 | the page, block or journal day does not exist |
| `ambiguous` | 3 | a name matched several pages, or a write's `id::` UUID is on several pages (nothing is written) |
| `busy` | 4 | the owner is yielding or stopping; not admitted. Reads retry for 30 s first |
| `unknown-verb` | 2 | the owner does not know the verb (an older Tine) |
| `bad-args` | 2 | arguments invalid for the verb; also a query that does not parse |
| `frame-too-large` | 2 | the request line was over the cap |
| `malformed` | 2 | the request was not a request object |
| `too-large` | 2 | a read answer would exceed a result or page bound |
| `protocol` | 5 | the owner speaks another protocol version |
| `owner-mismatch` | 5 | the endpoint belongs to another graph (hash collision) |
| `owner-start` | 5 | `tine serve` did not come up, or the owner's endpoint does not answer, or the owner stopped answering a read |
| `owner-unsafe` | 5 | the owner namespace or endpoint path is unsafe: on a shared Windows machine another account pre-created or loosened the directory, or occupied the pipe name (a lingering predecessor name is retried for 3 s first) |
| `owner-lock` | 5 | the owner lock call itself failed (not contention) |
| `unsupported` | 5 | no CLI owner on this platform (iOS/Android and unshipped targets) |
| `stale` | 6 | a write's page or block changed: `--rev` no longer matches, a handle no longer resolves (or resolves to several blocks), or the page changed twice while the write ran. Not admitted; read again |
| `refused` | 6 | the owner refused; `detail.reason` says why (below). Nothing changed |
| `draft-failed` | 7 | a write was admitted but its draft could not be saved (disk full, app-data error); nothing changed |
| `unwritten` | 8 | a write was admitted and applied in the owner, but the file is not written yet; Tine keeps trying |
| `superseded` | 8 | a write applied, then a later edit replaced it before it was saved |
| `uncertain` | 8 | admitted, outcome unknown (the connection was lost after the request was sent; `tine serve --stop`) |
| `internal` | 1 | an owner bug or unnamed failure; the message leads with a diagnostic id |

Linux, macOS and Windows support all graph commands against the app or `tine serve`,
including automatic serve startup, app handover and `serve --stop`. Windows uses
the account-wide lock under Known Folder LocalAppData/tine-owners (resolved through
`SHGetKnownFolderPath(FOLDERID_LocalAppData)`) and the local-only
pipe `\\.\pipe\tine-<user-sid>-<16-hex-graph-hash>`. The owners directory is
created with an explicit protected DACL. Existing directories may
have inherited or unprotected DACLs: both explicit and inherited ACEs are checked
on every use. The owner must be the current user, SYSTEM or Administrators; reparse
points, null DACLs, unknown ACE types, and grants of write, delete, delete-child,
WRITE_DAC or WRITE_OWNER to anyone outside those principals refuse. Safe inherited
directories keep their ACLs; no permissions are repaired. New lock files also
get an explicit private protected DACL; existing lock files keep theirs. An
inherit-only CREATOR OWNER ACE is also safe on directories. The pipe
grants access only to the current user, rejects remote clients, and its first
instance retries an occupied name for 3 s while holding the owner lock, allowing
a predecessor's handles to drain, then refuses it. A busy pipe is waited on up to
the caller's I/O deadline; it is never reported as no owner. Before sending a hello
the client checks the connected pipe's owner and protected DACL. Unsafe permissions or pipe creation
conflicts report `owner-unsafe` (exit 5). Same-account processes are trusted.
Framing, caps, deadlines, custody checks, the 60 s handover bound and the rule that
a timeout never permits takeover are shared with Unix. On desktop, a namespace or
lock failure keeps the graph editable without a lease, with a persistent,
non-dismissible warning naming single-writer protection and the command line,
the cause, recovery instructions, Retry now, and copyable diagnostics. An owned,
non-symlink Unix owners directory can be explicitly tightened to mode 0700;
the repair preserves existing lock inodes. Foreign directories, symlinks and
Windows DACLs are never repaired by this action. The app retries every 3 seconds
after an attempt finishes, through the same custody-checked handover (up to
60 seconds per attempt); a newly started `serve` must yield and release its
lock before the app claims it. A timeout or refusal leaves the warning and
unleased editing in place. Successful acquisition clears the warning.

Lock discovery does not apply the Unix socket pathname limit. Missing Unix state
parents are requested with mode 0700; existing parent permissions are preserved.
An overlong account home therefore still permits the app to hold its durable
account-wide lock. Endpoint binding then fails separately, retaining the lease;
CLI discovery reports `owner-unsafe` (exit 5) with the endpoint pathname limit
before attempting to start another host. There is no alternate temporary or
environment-dependent lock namespace. A bind failure after acquisition keeps
the lock and does not enter the unleased retry path. iOS/Android
have no CLI and rely on one process per sandboxed app.
Failed arbitration remains a limit: two hosts may overlap from the failed claim
until acquisition/handover succeeds, including its wait. Persistent failure or
handover refusal can make that interval indefinite. Revision guards and durable
publication remain, but do not replace cross-process owner serialization.
See `docs/storage-contract.md` (L9-01/L9-02).

The client bounds each reply frame at 60 s, including `serve --stop`; partial
bytes do not restart that deadline. A timeout leaves admitted work and the owner
lease with the owner. Reads may retry within their retry window; a write or stop
whose reply is lost reports `uncertain` (exit 8) and is never resent automatically.

**Refusal reasons** (`detail.reason`, and `refused:<reason>` in the text):

| reason | when |
|---|---|
| `app-owner` | `tine serve --stop` to a graph the app owns |
| `draft-namespace` | a yield whose opening app keeps its drafts elsewhere |
| `unsaved` | the page has typed text Tine could not save; save or discard it in the app |
| `page-busy` | other work held the page; try again |
| `host` | the page host refused the write |
| `no-host` | no page host could be started for the write |
| `read-only` | the page is read-only in Tine (a Guide page, or a page that cannot round-trip) |
| `does-not-round-trip` | the new text would not read back as written (an unclosed fence, a line that becomes a new block) |
| `store` | the store refused the save plan |
| `cross-page` | `move` between two pages (not supported yet) |
| `broken-ref` | `add` content names a `((uuid))` no block has (`detail.uuid`) |
| `page-exists` | page creation or rename would replace an existing file or alias |
| `confirmation` | `page delete` needs `--yes` or `--force` |
| `rename-options` | graph-wide `page rename` cannot use the single-file `--rev` guard or run commits |
| `git` | a run verb or `--run` write: the graph's git repository cannot take the run's commits (none, bare, a merge/rebase in progress, a detached HEAD, an ignored file, a libgit2 error) |
| `git-busy` | a `--run` write while another git command holds the index (before any change), or inside a `git` object: the branch kept moving while a run write committed |
| `merge` | `run.revert`: a page's Concord merge refused it (nothing changed); inside a page entry, the same for a page that changed between the check and its write |
| `kernel` | the edit itself cannot apply to the block (message says why) |
| `kernel:<reason>` | the outline refused the verb: `kernel:depth_cap`, `kernel:cycle`, `kernel:read_only`, `kernel:frozen`, `kernel:cross_page_merge`, `kernel:header_root`, `kernel:no_outline_step`, `kernel:incomplete`, `kernel:unparseable` |

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
| `cli-write-failed` | a write's plan failed on an I/O error, or planned no bytes |
