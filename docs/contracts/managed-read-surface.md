# Managed read surface

This contract pins the production `application_*` family. A live row is either
`necessary` because it owns a Managed-only input/index boundary, or an `adapter`
whose body delegates to the named shared owner. New twins are prohibited by
I-12 and D-4. The syntactic guard uses
`projection_producer_census::production_rust()` and verifies membership and
adapter call edges.

## Live family

| symbol | file | class | canonical owner | justification/evidence |
| --- | --- | --- | --- | --- |
| application_page_block_referrers | `crates/tine-core/src/query.rs` | adapter | collect_reference_matches | Walks the page's projected forest through the shared reference visitor; owns no traversal of its own. |
| application_page_property_dto | `crates/tine-core/src/query.rs` | necessary | — | Projects one Managed page-property DTO input. |
| application_page_property_owner_rows | `crates/tine-core/src/query.rs` | necessary | — | Owner-preserving property rows of one Managed overlay page: the property registry's overlay source (SPEC §6.2). Owner identity is exactly what the pair form drops. |
| application_page_property_pairs | `crates/tine-core/src/query.rs` | adapter | application_page_property_owner_rows | Normalizes Managed wire property pairs by projecting the owner rows to `(key, value)` — one producer, not a twin. |
| application_page_reference_matches | `crates/tine-core/src/query.rs` | adapter | collect_reference_matches | Matches references over a page's projected forest through the shared reference visitor. |
| application_page_templates | `crates/tine-core/src/query.rs` | adapter | visit_template_blocks | Walks Managed DTO shape and delegates each result to the canonical template leaf. |
| application_query_doc_block | `crates/tine-core/src/query.rs` | necessary | — | Rehydrates a complete Managed DTO subtree. |
| application_sparse_query_doc_block | `crates/tine-core/src/query.rs` | necessary | — | Rehydrates a sparse materialized-query row through `DocBlock::new`. |
| application_advanced_query_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed advanced-query transaction boundary. |
| application_all_query_pages_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed all-pages query index boundary. |
| application_backlink_filter_context_ready | `crates/tine-core/src/sync_runtime.rs` | adapter | backlink_filter_entry | Hydrates cached roots and delegates entries to the shared DocBlock producer. |
| application_backlinks_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed backlink candidate/index boundary. |
| application_block_candidates_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Merges SQLite and pending UUID claimant sets. |
| application_block_children_by_identity | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Locates children in a Managed editor DTO tree. |
| application_block_reference_counts_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Merges Managed reference-count index and overlay. |
| application_block_referrers_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Merges Managed referrer index and hydrated pages. |
| application_editor_blocks | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Sole Managed save-request block DTO constructor. |
| application_editor_blocks_existing | `crates/tine-core/src/sync_runtime.rs` | adapter | application_editor_blocks | Existing-save exposed-key policy delegates to the shared builder. |
| application_editor_blocks_new | `crates/tine-core/src/sync_runtime.rs` | adapter | application_editor_blocks | New-save generated-key policy delegates to the shared builder. |
| application_equivalent_page_names_ready | `crates/tine-core/src/sync_runtime.rs` | adapter | equivalent_page_names | Supplies Managed name-index candidates to the shared equivalence rule. |
| application_export_query_subtrees_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed export hydration and subtree budget boundary. |
| application_from_clean_foreground_commit | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Converts a Managed foreground commit result. |
| application_fuzzy_candidate_paths_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed fuzzy-name index boundary. |
| application_graph_search_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed search index and pending-overlay merge. |
| application_hydration_cache_budget_for_available | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed hydration-cache resource boundary. |
| application_hydration_retained_bytes | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed hydration-cache accounting boundary. |
| application_inventory_of_kind_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed kind-filtered inventory index. |
| application_inventory_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed materialized inventory plus overlay. |
| application_journal_feed | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed journal-feed state owner. |
| application_journal_feed_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed journal index plus pending overlay. |
| application_journal_naming | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed graph-config journal naming input. |
| application_load_outcome | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Converts exact Managed load state and revision. |
| application_materialized_read_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Opens the exact Managed SQLite frontier. |
| application_move_accepted | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed move durable-acceptance boundary. |
| application_move_batch_id | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed move batch identity boundary. |
| application_move_committed_outcome | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Converts a committed Managed move result. |
| application_move_request_digest | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Digests the Managed move request representation. |
| application_navigation | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Handle-side Managed navigation request boundary. |
| application_navigation | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Actor-side Managed navigation dispatch boundary. |
| application_navigation_aliases_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed alias index plus overlay. |
| application_navigation_overlay_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Decodes the committed-undrained Managed suffix. |
| application_navigation_pages_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed page inventory index plus overlay. |
| application_navigation_reference_names_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed reference-name index plus overlay. |
| application_orphan_assets_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed asset-reference index boundary. |
| application_page_block_reference_counts | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Counts references in a hydrated Managed page. |
| application_page_identity_map | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Maps Managed editor identities for stable saves. |
| application_page_inventory | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Builds inventory from one hydrated Managed page. |
| application_page_inventory | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Builds inventory from one Managed projected page. |
| application_page_namespace_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed page namespace index boundary. |
| application_page_rename_sources_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed rename-source index boundary. |
| application_page_request_too_large | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed editor admission boundary. |
| application_pages_at_name_key_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed exact-name index boundary. |
| application_parser_indices_for_block_ids | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Maps Managed block IDs to parser positions. |
| application_preview_block_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed bounded-subtree preview boundary. |
| application_projection_roots | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Owns the cached complete DocBlock view for a Managed page. |
| application_property_facets_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed property-facet index plus overlay. |
| application_property_registry | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Owns the Managed-only refusal policy for the property registry: a refused materialized read serves the last published snapshot rather than a half-built table (D-3 recovery). |
| application_property_registry_cache_key | `crates/tine-core/src/sync_runtime.rs` | necessary | — | The evidence stamp a registry snapshot was built from — accepted frontier pair plus `ParseConfig::digest()` — so an unchanged graph reuses the snapshot instead of rebuilding it per query (SPEC §6.2). Managed-only: it encodes the acceptance-sequence/pending-suffix rule the Direct Files side has no analogue for. |
| application_property_registry_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed property-registry snapshot: the materialized owner-row stream masked by the unaccepted local overlay, merged with the overlay's own rows (SPEC §6.2 C4). |
| application_property_registry_snapshot_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | SPEC §7.1 `query_registry` on the Managed side: the wire snapshot of the merged registry, carrying `application_property_registry`'s refusal policy (the last published table, never a half-built one). |
| application_query_explain_empty_ready | `crates/tine-core/src/sync_runtime.rs` | adapter | explain_application_empty_query | SPEC §7.1 `query_explain_empty` (Q14, N19): assembles the Managed page set and calls the shared computation. |
| application_query_page_journal | `crates/tine-core/src/sync_runtime.rs` | adapter | parse | Calls `crate::date::JournalFormat::parse` (the census records a call by its last path segment) to read the page's journal day from the graph's CONFIGURED format — the same producer that fills Direct's `PageEntry::date_key`. Never `JournalDate::from_title`, which hardcodes the default format. |
| application_query_page_recency | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Computes recency from Managed path and graph config. |
| application_query_plan_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed query-plan/index preparation boundary. |
| application_query_run_ready | `crates/tine-core/src/sync_runtime.rs` | adapter | run_application_query_result | SPEC §7.1 `query_run`: the IR arrives parsed, so only the Managed page assembly is mode-specific; the evaluator is the shared one. |
| application_request | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Handle-side Managed application request boundary. |
| application_resolve_blocks_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Materializes resolved Managed UUID groups. |
| application_simple_query_pages_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed simple-query candidate index boundary. |
| application_simple_query_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed simple-query execution boundary. |
| application_sparse_task_query_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed task-index sparse hydration boundary. |
| application_subtree_nodes | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Counts nodes in a Managed DTO subtree for admission. |
| application_templates_ready | `crates/tine-core/src/sync_runtime.rs` | adapter | application_page_templates | Supplies hydrated Managed pages to the canonical template walk. |
| application_unit_page_home_hints | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed unit-transaction page-location hints. |
| application_unlinked_candidate_strategy | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Chooses the Managed unlinked-reference index strategy. |
| application_unlinked_references_ready | `crates/tine-core/src/sync_runtime.rs` | necessary | — | Managed unlinked-reference candidate/index boundary. |
| application_blocks_have_content | `src-tauri/src/commands.rs` | necessary | — | Command adapter consumes application DTO trees without mode branching. |
| application_property_line | `src-tauri/src/commands.rs` | necessary | — | Command adapter parses application DTO property lines. |
| application_page_admission | `src-tauri/src/state.rs` | necessary | — | App-state admission boundary for Managed page payloads. |

## Retired producers

| symbol | former file | canonical replacement | packet item |
| --- | --- | --- | --- |
| application_backlink_filter_entry | `crates/tine-core/src/query.rs` | backlink_filter_entry | C7a-1 |
| template_dto_from_application | `crates/tine-core/src/query.rs` | template_dto | C7a-3 |
| application_crumb_line | `crates/tine-core/src/sync_runtime.rs` | crumb_line | W4-C7b-1 |

## UUID ownership policy

| boundary | exact outcome/rule | OG commit | OG path |
| --- | --- | --- | --- |
| Direct-ready | Physical projection is a hint; ambiguity falls back and parser-order first claimant owns the UUID. | c67b8b5fa47f8fe1e1954226c9bdfabd46ebb968 | `deps/graph-parser/src/logseq/graph_parser/block.cljs` (`fix-block-id-if-duplicated!`) |
| Direct-fallback | Parser-order first claimant owns the UUID. | c67b8b5fa47f8fe1e1954226c9bdfabd46ebb968 | `deps/graph-parser/src/logseq/graph_parser/block.cljs` (`fix-block-id-if-duplicated!`) |
| Managed-pending | Merge exact overlay and SQLite pages in graph path/tree order; first claimant owns the UUID. | c67b8b5fa47f8fe1e1954226c9bdfabd46ebb968 | `deps/graph-parser/src/logseq/graph_parser/block.cljs` (`fix-block-id-if-duplicated!`) |
| Managed-drained | A unique SQLite hint is accepted; ambiguity is resolved from pages in graph path/tree order, first claimant. | c67b8b5fa47f8fe1e1954226c9bdfabd46ebb968 | `deps/graph-parser/src/logseq/graph_parser/block.cljs` (`fix-block-id-if-duplicated!`) |

OG also declares `:block/uuid` unique identity in
`deps/db/src/logseq/db/schema.cljs`; the parser establishes which claimant keeps
the UUID before that identity enters the database.

## Template extraction policy

| boundary | exact outcome/rule | OG commit | OG path |
| --- | --- | --- | --- |
| Property extraction | Normalize property keys to lowercase and replace slash, spaces, and underscores with hyphens. | c67b8b5fa47f8fe1e1954226c9bdfabd46ebb968 | `deps/graph-parser/src/logseq/graph_parser/block.cljs` and `deps/graph-parser/src/logseq/graph_parser/extract.cljc` |
| Template discovery | A parsed block whose normalized property map contains `template` is a template; ordinary page pre-block text is not a block template. | c67b8b5fa47f8fe1e1954226c9bdfabd46ebb968 | `src/main/frontend/db/model.cljs` (`get-all-templates`) |

## Measured exception

`shallow_application_block` remains at exactly four syntactically pinned
Managed result boundaries (W4-C7b retired the fifth with the managed
block-referrer twin). The DTO already contains parser-derived facets;
routing through `dto_block_to_doc_block` plus `block_to_shallow_dto` creates one
temporary `DocBlock` per result and reparses raw text. See
`managed_shallow_application_block_manual_probe` and
`measurement-shallow-application-block.txt`. This is a measured keep under D-4,
not permission for another shallow producer.

## `BlockDto::id` is a runtime handle, not a durable identity

The two modes put different values in this one wire field, and both are
correct:

| mode | `BlockDto::id` | durable `id::` |
| --- | --- | --- |
| Direct | the projection's `block_id` (a runtime identity; `direct_projection.rs` keeps `logseq_uuid` as a separate column) | in `properties`, unchanged |
| Managed | the durable `id::` uuid where the block has one | in `properties`, unchanged |

Nothing user-facing depends on the difference: every reference-producing path
goes through `ensureBlockId` (`src/store.ts`), which reads the existing `id::`
out of the block's raw text — case-insensitively — and never writes a second
one, precisely so a copied `((ref))` cannot dangle.

**The rule for new consumers:** treat `BlockDto::id` as a handle valid for the
current session and mode only. Anything that persists, exports, or compares a
block identity across modes must read the `id` entry of `properties`. The
real-graph gate `managed_c7a_real_graph_copy_manual_gate` normalizes this field
out of its parity oracle for exactly this reason; if a future packet needs the
two modes to agree on it, that is a deliberate identity change, not a bug fix.

## W4-C7b measured collapse

One evaluator per question, measured before and after on the same machine. The
gate is the structural column — how many production functions own the algorithm.
Timing is advisory: this machine builds other worktrees concurrently.

Runtime counters come from `c7b_measure_managed_and_direct_reads` (30 indexed
pages + 5 journals, one cold pass and nine warm passes) and
`c7b_measure_anonymized_corpus` (1,045-page real graph, one cold pass and four
warm). `allocation_calls` / `allocated_bytes` are a `#[cfg(test)]` counting
global allocator defined in `sync_runtime_tests.rs::c7b_alloc`; no allocation
counter existed anywhere under `crates/` to reuse.

| function | file | question | classification | shared producer | callers | before structural counters | after structural counters | timing (advisory) | exception rationale / evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `run_query_bounded` / `run_application_query_pages_bounded` | `crates/tine-core/src/query.rs` | which blocks satisfy a simple query, bounded | cut | `run_query_bounded_over` over `QueryPageSource` | 2 | 2 algorithm owners | 1 algorithm owner | managed warm simple query unchanged at the memo (859 alloc calls, 269 µs median) | `c7b_query_driver_producers_share_one_page_source` |
| `run_pred_bounded` / `run_application_pred_pages_bounded` | `crates/tine-core/src/query.rs` | which blocks satisfy a parsed predicate, bounded | cut (the managed twin deleted outright) | `run_pred_bounded_over` | 2 → 1 | 2 algorithm owners | 1 algorithm owner | n/a (shared with the driver above) | `c7b_query_driver_producers_share_one_page_source` |
| `run_advanced_query_bounded` / `run_application_advanced_query_pages_bounded` | `crates/tine-core/src/query.rs` | which blocks satisfy an advanced query, bounded | cut | `run_advanced_query_bounded_over` | 2 | 2 algorithm owners | 1 algorithm owner | not separately timed; same driver | `c7b_query_driver_parity` |
| `export_query_subtrees` / `export_application_query_subtrees` | `crates/tine-core/src/query.rs` | which subtrees a query exports, bounded | cut | `export_query_subtrees_over` | 2 | 2 algorithm owners, 2 budget constants | 1 algorithm owner, 1 budget constant | not separately timed | `c7b_query_driver_parity` |
| `collect_reference_occurrences_bounded`, `bound_application_reference_sources`, the managed block-referrer loop | `crates/tine-core/src/query.rs`, `crates/tine-core/src/sync_runtime.rs` | how reference rows are grouped, admitted and ordered | cut | `BoundedReferenceGroups` + `reference_group_display_order` | 3 | 3 algorithm owners, 3 display-order copies | 1 algorithm owner, 1 display-order producer | see the reference row below | `c7b_reference_grouping_has_one_algorithm_owner`, `c7b_bounded_reference_grouping_parity` |
| `application_page_reference_matches`, `application_page_block_referrers` | `crates/tine-core/src/query.rs` | which blocks of one page reference a target | cut | `collect_reference_matches` over cached `DocBlock` roots | 7 managed call sites | 0 of 7 reference reads used the retained projection; the block-referrer twin parsed every block twice | 7 of 7 use `application_projection_roots`; one `crumb_line` producer | 30 pages warm: 119,882 → 87,417 alloc calls, 15.15 → 11.48 MB, 82.5 → 63.2 ms median. Corpus warm: 461,493 → 367,446 alloc calls, 291.2 → 238.5 ms median, cache hits 0 → 168, misses 0 | `c7b_managed_reference_reads_use_shared_docblock_visitors`, `measurement-before.txt`, `measurement-after.txt`, `measurement-corpus-*.txt` |
| `application_navigation_overlay_ready` consumers | `crates/tine-core/src/sync_runtime.rs` | what the pending local suffix contains, for this request | cut | one request-scoped `navigation_overlay_scoped` memo | 6 `*_ready` consumers | QuickSwitch: 3 pending-path load passes per request | 1 | not separately timed; the counter is the gate | `c7b_navigation_overlay_pending_paths_load_at_most_once_per_request` |
| `journal_ordinal` | `crates/tine-core/src/query.rs` | which day a journal page evaluates as | cut for the query adapters; **measured exception for the sparse path** | `JournalFormat::parse` via `ApplicationQueryPage::journal` | 2 → 1 | 2 producers, disagreeing on any custom `:journal/page-title-format` | 1 config-aware producer for both backends | none | The remaining `journal_ordinal` caller is the sparse task-candidate evaluator, whose `ApplicationSparseQueryPage` is also constructed in `direct_projection.rs`; BOTH backends read the same default-bound value there, so it cannot diverge. Collapsing it needs a file outside this packet's write set. `REG-W4-C7B-MANAGED-JOURNAL-ORDINAL-001` |
| Direct Files, all questions | `crates/tine-core/src/model.rs` | — | unchanged by construction | `GraphQueryPages` yields borrowed `(&meta, &[DocBlock], …)` | — | backlinks warm 6 alloc calls / 65 B / 1 µs; corpus warm 6 alloc calls / 2 µs | identical: 6 alloc calls / 65 B / 1 µs; corpus 6 / 2 µs | no change | Direct constructs no page forest and clones no root vector for a query, before or after; `ApplicationProjectionCache` has zero Direct call sites |
