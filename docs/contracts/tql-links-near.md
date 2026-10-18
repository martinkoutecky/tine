# TQL link conditions: `links`, `linked_by`, `near`

Status: **prototype for review** (M3, vision §4.3 revision, D-20261004-V2d). The
spelling below is persisted in users' pages, so it is the one-way door; the rest
can move.

TQL is Tine's own query text (`{{tine-query …}}`). These conditions are
Tine-only: OG's `{{query}}` language is unchanged, and a query that uses one is
saved as `{{tine-query …}}` like every other Tine-only condition. They are
**page conditions**: they apply to `@page` rows and to the page elements of
another link condition. At `@block` they do not apply (the condition is kept and
reported, as `blocks` is at `@block`).

## Syntax

```
any(links, <page condition>)        every(links, …)       none(links, …)
any(linked_by, <page condition>)    every(linked_by, …)   none(linked_by, …)

near(<seed condition>, <k>)
near(<seed condition>, <k>, '<dir>')
near(<seed condition>, <k>, through => <page condition>)
near(<seed condition>, <k>, '<dir>', through => <page condition>)
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
    that link to the seeds, …); `'both'` is the union of the two, **each path
    keeping one direction** (k steps out plus k steps in, OG's graph-view
    meaning). It is **not** the undirected ball: with `'both'`, a page that
    links to a page the seed links to is not reached at k = 2. `'both'` is the
    default and is not printed.
  - `through => <page condition>`: a path may continue only through pages that
    satisfy the condition. A page that fails it **is still selected when a path
    reaches it, but no path continues from it** (include-but-stop, Neo4j APOC
    `subgraphNodes` termination filter). Seeds are always selected and always
    expanded, whatever `through` says. To leave pages out entirely, combine:
    `near(…, through => journal = false) and journal = false`.

The canonical printed form omits `'both'` and an absent `through`; a `through`
argument is always named. Every other spelling (a named `k`, `dir` without
quotes, `through` before `dir`, a `through` given twice) is a syntax error,
never a silently different query.

### Examples

```
{{tine-query @page and near(name = 'Project', 1)}}
    Project, the pages it links to, and the pages that link to it
    (= name = 'Project' or any(links, name = 'Project')
       or any(linked_by, name = 'Project')).

{{tine-query @page and near(name in ('Lemma 4', 'Draft'), 2, through => journal = false)}}
    Two steps out and two steps in from either seed; a journal day that links
    to a seed is selected but does not drag in everything else it mentions.

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
  and R explicit references; memory O(P + E) for E links (two `u32` adjacency
  entries per link plus one name per page). A later snapshot builds its own.
- **`near`** is breadth-first search over that table, one search per
  direction, never recursion: O(seeds + pages reached + links followed). The seed
  condition is evaluated once per page; `through` is evaluated once per page a
  search reaches.
- **`links` / `linked_by`** evaluate their element condition once per page, then
  each row reads its own adjacency: O(P + E) per condition.
- Nested link conditions are evaluated innermost first; each is computed once
  per query execution.
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
cycle, 2-cycle, hub, journal hub with `through` — fileless seeds, rename of a
seed inside a definition, and parity with the Linked References panel on the
kitchen-sink, `tine-test` and anonymized-graph copies).
