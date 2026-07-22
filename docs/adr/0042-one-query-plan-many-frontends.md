# 0042. One query plan, many frontends and presentations

- **Status:** Accepted
- **Date:** 2026-07-13

## Context

Tine currently has two overlapping selection engines. Ctrl+K parses a friendly
boolean/phrase/regular-expression dialect and combines fuzzy page navigation with
block search. `{{query}}` uses a richer predicate DSL and visual builder. The two
surfaces independently parse, match, diagnose, and present related operations;
JavaScript and Rust even compile regular expressions separately. Persistent
search (#99), better result evidence (#98), and understandable queries (#69)
would deepen that drift if implemented as more parallel systems.

At the same time, Ctrl+K is not only a query UI: commands and page creation are
launcher providers. Existing Logseq-compatible queries must retain their current,
non-fuzzy block semantics, and stored graph content should remain ordinary
`{{query ...}}` text rather than Tine-only temporary files.

## Decision

Tine will have one typed, Rust-authoritative `QueryPlan` for graph selection.
Friendly search text, the visual builder, compact chips, raw query DSL, and future
declarative plugin/NL frontends compile to that plan within their explicitly
declared lossless ranges. A frontend must never silently discard clauses it
cannot represent.

The plan distinguishes targets (pages, blocks, or both), predicate match modes
(including explicit fuzzy page-name matching), scope, ordering, limits, grouping,
and aggregation. Evaluation returns typed page/block hits with stable identity,
match evidence, diagnostics, and optional explanation data. Existing query DSL
defaults to block targets and non-fuzzy matching. Ctrl+K may execute a combined
page/block plan, but commands and page creation remain outside the query engine.

Block-result membership is identity-based. Query presentation transcribes
Logseq OG's `tree/filter-top-level-blocks` literally: a result is suppressed only
when its immediate parent is also present in the unfiltered result set. A matching
descendant below a non-matching intermediate block remains a separate result.
Reference occurrence surfaces do not reuse that presentation filter: every
referring block remains independently countable and navigable. Query, reference,
search, and batched block-resolution DTOs are shallow; list/embed renderers load
source hierarchy only where their presentation calls for it. A consumer that
genuinely needs an owned subtree (hover or export) must apply explicit node and
byte/work budgets before hydration and serialization. Native bridge commands
also enforce total row and byte budgets.

Search workspaces are virtual, graph-scoped, device-local routes that persist the
source expression rather than a result snapshot. They create no graph file until
the user names one. Naming performs a collision check, materializes one normal
page containing a canonical `{{query ...}}` block through the audited save path,
and replaces the route in place. Friendly search remains the primary authoring
surface; raw DSL is optional. Presentation stays orthogonal per ADR 0030, with a
search/excerpt view alongside list, table, and board.

## Consequences

Search and queries share semantics, diagnostics, evidence, and performance
profiles without forcing every surface to look alike. Regex behavior has one
authority. Fuzzy page matching becomes reusable and explicit rather than changing
old queries. Query workspaces can progress from transient words, through filters
and advanced construction, to a named Logseq-compatible dashboard without
polluting the graph.

The evaluator must still expose distinct execution profiles: capped/cancellable
preview, paged/virtualized workspace, existing inline-query behavior, and
on-demand explanation/count/timing work. The first fuzzy implementation is page
names only; fuzzy block content requires indexing or prefiltering before it can be
considered. Arbitrary scripting, bulk mutation, commands, and page creation do
not enter this model.

This result-shape contract is part of the performance model, not an optional
serialization optimization. Reintroducing recursive DTOs for every result would
make nested matches quadratic and would also multiply retained frontend state.
