# TQL link conditions: `links`, `linked_by`, `near`

Status: **decided syntax, Beta** (M3, vision §4.3 revision, D-20261004-V2d;
syntax decided by Martin 2026-10-18, D-20261018-T1). The spelling below is
persisted in users' pages, so it is the one-way door; the rest can move. On og
(Beta) only; stable takes it after conceptual model v0 defines links on nodes.

TQL is Tine's own query text (`{{tine-query …}}`). These conditions are
Tine-only: OG's `{{query}}` language is unchanged, and a query that uses one is
saved as `{{tine-query …}}` like every other Tine-only condition. They are
**page conditions**: they apply to `@page` rows and to the page elements of
another link condition. At `@block` they do not apply (the condition is kept and
reported, as `blocks` is at `@block`).

## Syntax

Live refresh on development (GX4 S4): mounted query readers register their
resolved plan with the host. Narrow queries refresh only when the memo's
page-local invalidation predicate says an edited page can affect them. Queries
over links/near, tag targets or the property registry coalesce relevant changes
until 1.5 seconds after the latest local edit, and wait for that edit's
publication before running. Tag-target and registry readers retain precise
invalidation for their respective inputs. Links/near remain conservatively
affected by every page edit, as the memo is. Full summaries, missed revisions
and unknown dependencies always invalidate; answers are fully recomputed.
Conservative `full` registrations (including Pages and blocks twins and graph
search) use the same quiet period because their work may be graph-wide. After
asynchronous registration renewal, readers recheck the latest input and
publication gates before starting execution, rearming when either gate moved.
Instant-dependent `now` plans invalidate on every publication: a currently
nonmatching page cannot prove that the displayed answer is still current.
Collapsed reference sections retain their last count, mark it as potentially
out of date, and fetch the current answer when opened.

```
any(links, <page condition>)        every(links, …)       none(links, …)
any(linked_by, <page condition>)    every(linked_by, …)   none(linked_by, …)

near(<seed condition>, <k>)
near(<seed condition>, <k>, '<dir>')        <dir> is 'out', 'in' or 'both'
```

- `links` — the pages this page links to. `linked_by` — the pages that link to
  this page. Inside the condition, `name`, `journal`, `day`, `namespace`,
  `used_as_tag`, `page_prop(…)`, `page_tag(…)`, `any(blocks, …)` and the link
  conditions themselves describe the OTHER page (the element), exactly as
  `any(blocks, …)` describes a block.
- `near` — the seed pages and every page within `k` link steps of one.
  - `<seed condition>` is any page condition. Seeds are usually named:
    `name = 'Project'`, `name in ('A', 'B')`.
  - `<k>` is a whole number literal, `0` or more. `0` selects the seeds alone.
  - `<dir>` is a quoted string: `'out'` follows links forward (pages the seeds
    link to, then pages those link to, …); `'in'` follows them backwards (pages
    that link to the seeds, …). `'both'`, the default, is not printed.

### `'both'` is not the undirected ball

`'both'` is the **out-neighbourhood at distance k together with the
in-neighbourhood at distance k**: one search forward, one search backward, and
**each path keeps one direction**. It is *not* the undirected ball (every page
within k steps when a path may switch direction at every step), which is what
an undirected or "both" step means in Cypher (`-[*1..k]-`), Gremlin
(`both()`) and SPARQL property paths (`(p|^p)`). With `'both'`, a page that links to a page the seed links to is not
reached at k = 2 (seed → x ← z: z is reached only by switching direction).
Martin, 2026-10-18 (D-20261018-T1): a page linking to a page you link to may be
unrelated. The undirected shape is not expressible in one `near` today.

### Hub control is reserved

There is no hub control (no way to stop a path at a page such as a busy
journal day). `near` takes **at most three positional arguments**; a fourth
argument, and any named argument (`… => …`), is a syntax error whose message
says the slot is reserved for a later version. The prototype's
`through => <condition>` is such an argument and is refused the same way. A
conjunct removes rows but never stops a path: `near(name = 'Project', 2) and
journal = false` still reaches the pages a journal day links to.

The canonical printed form omits `'both'`. Every other spelling (a named `k`,
`dir` without quotes, a fourth argument) is a syntax error, never a silently
different query.

No TQL function (`near`, `any`/`every`/`none`, `ref`, `prop`, `off`, …) takes
the SQL function modifiers the grammar would accept: `FILTER (WHERE …)`,
`OVER …`, `WITHIN GROUP (…)`, `IGNORE/RESPECT NULLS`, `DISTINCT`/`ALL`, an
`ORDER BY`/`LIMIT` inside the arguments, a subquery argument, or named and
wildcard arguments outside `near`'s reserved slot. Each is a syntax error,
never dropped.

### Examples

```
{{tine-query @page and near(name = 'Project', 1)}}
    Project, the pages it links to, and the pages that link to it
    (= name = 'Project' or any(links, name = 'Project')
       or any(linked_by, name = 'Project')).

{{tine-query @page and near(name in ('Lemma 4', 'Draft'), 2) and journal = false}}
    Two steps out and two steps in from either seed, journal days left out of
    the rows (paths still go through them).

{{tine-query @page and near(name = 'Paper', 3, 'out')}}
    Everything Paper links to, up to three steps away.

{{tine-query @page and any(links, name = 'Reading list') and journal = false}}
    Pages (not journal days) that link to Reading list.

{{tine-query @page and none(linked_by, true)}}
    Orphans: pages nothing links to.

{{tine-query @page and near(page_tag('project'), 1) and not name = 'Archive'}}
    A slice: every project page and its neighbours, minus one page.
```

## What a link is

A pair (S, T) is a link exactly when page S appears in page T's **Linked
References** panel (unfiltered, as shown when T is opened under its own name).
The table is built by the panel's own rules, so the two cannot disagree:

- the referring text is any block of S, including its page-properties header
  (`tine_core::reference_evidence` decides what is an explicit reference:
  `[[T]]`, `#T`, `#[[T]]`, linkable property values such as `tags::`, task
  markers and enabled property-key pages; a reference to `a/b` is also a
  reference to `a`);
- T is matched by its name, its aliases and, for a journal day, every accepted
  spelling of that day (`graph_equivalent_page_names`);
- S is never T itself and never the Favorites arrangement page
  (`refs::ReferenceSourceExclusions`).

Unlinked references (plain mentions) are not links. A block reference
`((uuid))` is not a page link.

**Pages without a file.** The panel exists for a page that is only linked to,
so such a page is a node too: it can be a seed (`near(name = 'idea', 1, 'in')`
selects the pages linking to a fileless `idea`) and an element (`any(links, name
= 'idea')`). It has its name, `journal = true` and its `day` when the name is a
date, no properties and no blocks, and it never links anywhere. Like every
`@page` query, the result lists only pages that have a file.

## Renames

Seeds and element conditions are ordinary page conditions, so the existing
rename rule applies unchanged (`docs/contracts/export-definitions.md`): inside an
export definition block, renaming a page rewrites a quoted page name of an
active `name` comparison (`=`, `!=`, `in`, `not in`), wherever it sits —
including inside `near(…)` and `any(links, …)`. An ordinary query block keeps
its text, as in OG.

## Evaluation and cost

- **The link table.** Built once per graph snapshot when a query first needs it
  (`tine-store` `query::link_graph`), in memory, never persisted: one pass over
  every block's parser-owned reference spans, O(P + B + R) for P pages, B blocks
  and R explicit references. Retained memory is O(P + E) for E distinct links:
  two `u32` adjacency entries per link (each list is deduplicated and shrunk to
  its length, so a page naming one target many times holds one entry), one key
  per physical page and one name per page without a file. During the build the
  transient memory is O(R + E): one pending entry per distinct target of each
  page, and adjacency lists that receive each link once per source before the
  final sort, never R times an alias's owner count (200 owners of one alias
  named 50,000 times peak at about 1 MB above the start of the build for a
  338 KB table; before review r1 the occurrences were expanded first, +72 MB). Measured on the
  realistic 10k graph: 13,904 nodes, 94,121 links, about 2.3 MiB retained. A
  later snapshot builds its own.
- **`near`** is breadth-first search over that table, one search per
  direction, never recursion: O(seeds + pages reached + links followed). The seed
  condition is evaluated once per page.
- **`links` / `linked_by`** evaluate their element condition once per page, then
  each row reads its own adjacency: O(P + E) per condition.
- Nested link conditions are evaluated innermost first; each is computed once
  per query execution.
- **Cancellation.** `WholeGraph::query_ir(request, &Cancel)` takes the caller's
  flag (the `Cancel` flag interactive search takes). The row scan checks it
  once per candidate page; the table build, every element and seed condition
  check it at least once per page; the table build also once per 1,024
  references read and per 1,024 links expanded or indexed, and every `near`
  search once per 1,024 edges scanned (a hub with a million links is not one
  step). A cancelled run answers `Ok(None)`: no partial
  answer, no partial table, and a cancelled run is never served from the memo
  (the flag is read before the memo) nor stored into it (the flag is read
  again after the computation, before the store). In the app every
  `query_run`, `query_explain_empty`, `tagged_pages` and owner `query` request
  owns a fresh flag, registered on its window's graph binding until it
  returns; revoking the binding (graph switch, window close) sets all of them,
  and the command answers `query-cancelled: …`. A newer run of the same query
  block does not cancel an older one yet (the frontend shares in-flight runs
  between blocks by key; follow-up).
- **Bounds.** A link query is bounded like every page query (20,000 rows /
  32 MiB). A k = 2 search through hub pages can select most of a graph; that is
  the correct answer, not an error. `k` larger than the graph's depth selects
  everything reachable.
- **Memo.** An answer that reads the link table depends on every page, so any
  page edit drops it (the page-local scoped invalidation does not apply).

Unit cost (I-25): no new persisted record, index, file or transport record. Per
edit: 0 bytes written and 0 files beyond the edit itself, on a 1-block and on a
60-block page alike; transport bytes per edit: 0. The in-memory table is rebuilt
lazily on the first link query after an edit, O(P + B + R).

## Refusals

None: these conditions only read. A malformed `near(…)` is a syntax diagnostic
on the query, like any TQL error.

Tests: `crates/tine-core/src/query/tql_tests.rs` (`links_near_*`: spelling,
round trip, diagnostics, rename literals),
`crates/tine-store/tests/link_graph.rs` (direction cases on a hand graph —
cycle, 2-cycle, hub, journal hub, `'both'` versus the undirected ball —
fileless seeds, rename of a seed inside a definition, and parity with the
Linked References panel on the kitchen-sink, `tine-test` and anonymized-graph
copies), `crates/tine-store/src/query/link_graph.rs` `cancellation_tests` and
`query/memo.rs` (cancellation, the hub and alias fan-out checkpoints, and the
memo guard), `crates/tine-store/tests/link_graph.rs`
`links_table_build_memory_follows_references_plus_links_not_their_product`
(peak build memory), `tql_tests.rs`
`every_function_refuses_sql_function_modifiers`, `src-tauri/src/state.rs`
(`revoking_a_binding_cancels_its_running_queries_and_every_later_one`).
