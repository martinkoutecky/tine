# One-page managed save: the whole-graph catalog copy is gone, the graph-sized cost is not

**Outcome: one proven cut landed; frozen contract item 2 is not met.** The
warm one-page draft no longer reproduces the whole-graph catalog, and the
draft derivation's own work is now identical at every graph size. The draft
as a whole is still proportional to total graph pages, because what remains
is not in the draft: every authenticated scratch point read costs bytes
proportional to the graph. That is the archive-storage layer this dossier
excludes. The exact boundary and the decision it needs are at the end.

- Base: `272a3726`, branch `perf/managed-save-hot-draft`
- Commits: `75412eef` (production), `f135b243`, `1b44cb88` (proofs)
- Production diff: `crates/tine-core/src/oplog/hot_engine.rs` only

## The input report's named cause is not the cause

Contract item 1 required tracing the call chain rather than inferring it
from names. That was the right instruction: the named cause is wrong.

The report attributed 29% of save scaling to
`draft_admitted_local_author_transaction` pulling the catalog into the
author working set so that `snapshot_engine_documents` /
`derive_effect_from_snapshots` snapshot and diff it. Measured with per-phase
timers on the release receipt (local mode, 6 edits, 10 blocks/page):

| draft phase (per save, ms) | 100 pages | 1,000 pages | Δ | share |
| --- | ---: | ---: | ---: | ---: |
| **draft total** | 10.564 | 68.274 | **+57.71** | 100% |
| prepare core | 2.019 | 7.163 | +5.14 | 9% |
| ├ apply operations | 1.486 | 5.429 | +3.94 | 7% |
| ├ `snapshot_engine_documents` | 0.036 | 0.144 | **+0.11** | **0.2%** |
| ├ `derive_effect_from_snapshots` | 0.007 | 0.006 | **−0.00** | **0%** |
| └ frontier | 0.188 | 0.913 | +0.73 | 1% |
| **projection pages loop** | 8.544 | 61.111 | **+52.57** | 91% |
| ├ `materialize_page_for_projection` | 7.607 | 54.982 | +47.38 | 82% |
| │  └ of which catalog dependencies | 3.282 | 31.469 | +28.19 | 49% |
| └ `prospective_projection_page` | 0.915 | 6.122 | +5.21 | 9% |

The catalog snapshot/diff the report named is **0.2% of the draft's
scaling**. For an ordinary block edit the catalog never enters the author
working set at all: `apply_author_operation` only calls
`ensure_working_document(catalog)` for `CreatePage`, `EditPagePath`,
`SetPageKind`, `DeletePage`, `RenamePagesAndRewriteReferrers` and
`ReconcileExternalPageState`, and `page_home_from_working` resolves a page
home from an authenticated point row without the catalog. Counters agree:
`catalog_page_entry_visits` (rows decoded by whole-catalog enumeration) is
**0** on a warm content edit, at every graph size, on base.

The real whole-catalog reproduction in the draft was one line down the other
branch: `prospective_projection_page` (`hot_engine.rs:13252` on base) opened
with `self.prospective_document(self.catalog_document_id, prospective)`,
which for a document the transaction did not touch falls through to
`clone_visible_document` — a full export/import of the catalog CRDT, or a
full authenticated load of it from the scratch store. One per affected page,
every save.

## What changed

`prospective_document_ref` replaces the unconditional copy. It resolves the
catalog **in place** when, and only when, the drafted transaction's own
prospective document set does not contain the catalog:

```rust
if document_id == self.catalog_document_id
    && !prospective.contains_key(&document_id)
    && !self.uses_previous_document_derivation()
{
    if let Some(catalog) = self.current_catalog_document()? {
        return Ok(ProspectiveDocument::Borrowed(catalog));
    }
}
Ok(ProspectiveDocument::Owned(
    self.prospective_document(document_id, prospective)?,
))
```

`prospective_projection_page` then reads through the existing
`materialize_page_from_document_lookup` and
`resolve_logseq_uuid_from_document_lookup` seams, so no second graph model is
cached and no reader gained a new authority.

Why this is exact rather than a heuristic. `prospective` is the engine's own
clone of the transaction's working set. A document absent from it produced no
CRDT update in this batch, so its prospective state *is* its current visible
state — which is precisely what the old fallback asserted by loading that
same visible lane. `current_catalog_document()` is the same hot,
anchor-validated reader that already produces this draft's pre-state in
`materialize_page_inner`, so both sides of one draft now see one catalog
authority instead of two copies of it. Any engine that reaches this line has
already resolved the catalog through that reader inside
`materialize_page_for_projection`; when it yields `None`, the code falls
through to the previous derivation.

The selector is authenticated typed engine state only. No request DTO, no
caller assertion, no operation classification by name.

## Fallback boundary

The previous derivation is retained whenever the drafted transaction owns the
prospective catalog, which is every transaction that can move catalog-wide
page identity. Proved by `every_local_author_transaction_class_derives_the_
same_draft_as_the_previous_derivation`, which asserts
`optimized_catalog_copies == oracle_catalog_copies >= 1` for: new page, path
change, kind change, page deletion, namespace rename with referrer and
preamble rewrites. Page-local classes assert `optimized_catalog_copies == 0`
with `oracle_catalog_copies >= 1`.

Refusals stay identical in both derivations: duplicate block id, unknown
page, unknown block, and a duplicate accepted Logseq claim (which is refused
by the changed function itself, through the claim-participant home documents
it now resolves through the lookup seam).

## Fail-before and pass-after

`warm_one_page_content_edit_draft_is_independent_of_total_graph_pages` runs
the same warm content edit against a 4-page and a 40-page graph on an
enrolled, scratch-backed engine and compares every term the draft derivation
controls.

Before (test-only switch restoring the previous derivation, which is the base
code path byte for byte):

```
left  (4 pages):  copies: 2, copy_ops: 83,  catalog_copies: 1, catalog_rows: 0, scratch_reads: 45
right (40 pages): copies: 2, copy_ops: 119, catalog_copies: 1, catalog_rows: 0, scratch_reads: 45
FAILED: a warm one-page content edit must do the same draft work at every graph size
```

`copy_ops` rises by exactly one CRDT operation per extra page: the catalog.

After:

```
both scales:      copies: 1, copy_ops: 79,  catalog_copies: 0, catalog_rows: 0, scratch_reads: 42
ok
```

One document reproduced instead of two, the graph-proportional term gone,
three fewer authenticated scratch page reads per draft, and identical at both
graph sizes.

## Differential proof

`assert_draft_matches_previous_derivation` drafts one transaction twice on
one engine — nothing is captured, so the second draft starts from the same
state — and compares the typed drafts: author, origin, generation, root
token, prepared core (manifest, causal dot, dependency frontier, semantic
effect digest, object descriptors), semantic effect, portable path root,
projection requirements, prospective document set and each document's version
vector, and for each affected page its pre-state, post-state and accepted
frontier (materialized page, blocks, claim evidence). No digest snapshots.

Covered transaction classes: Markdown content edit, Org content edit, insert
block, reorder block, delete subtree, cross-page move subtree, page preamble,
Logseq identity claim, multi-operation editor save, plus the catalog-changing
and refusing classes listed above. Content carries task markers, properties
and `[[page]]` references so the claim/reference path is exercised, and pages
live at nested non-ASCII paths.

`in_place_catalog_derivation_publishes_the_same_markdown_and_org_source`
drives the complete local publication for `content/pages/研究/über topic.md`
and its `.org` twin: each transaction is first proved equivalent to the
previous derivation, then published for real and compared byte for byte
against the projected source, for a content edit, an inserted block, a
publication that only settles on the durable retry, and a restart that must
replay to the same source.

## Release comparison

Reduced recipe from the causal report: local and shared mode, 100 and 1,000
pages, 10 blocks/page, 6 edits, release, temporary phase timers, one binary
with the previous derivation behind a switch.

| local mode, per save | before 100 | before 1,000 | after 100 | after 1,000 |
| --- | ---: | ---: | ---: | ---: |
| public `save_application_page` p50 | 61.297 ms | 258.974 ms | 64.745 ms | 238.514 ms |
| public p95 | 92.537 ms | 411.052 ms | 91.904 ms | 378.056 ms |
| draft phase | 10.564 ms | 68.274 ms | 10.892 ms | 63.531 ms |
| **`prospective_projection_page`** | **0.915 ms** | **6.122 ms** | **0.608 ms** | **3.109 ms** |
| `materialize_page_for_projection` | 7.607 ms | 54.982 ms | 8.245 ms | 53.527 ms |
| authenticated document point reads | 15/save | 15/save | 14/save | 14/save |

The cut halves the phase it targets: `prospective_projection_page`'s
graph-sized term falls from +5.21 ms to +2.50 ms (−52%), and one whole-document
authenticated read per save is gone at both scales. The draft's total
graph-sized term falls from +57.71 ms to +52.64 ms (−8.8%).

At the public boundary the change is not distinguishable from noise: p50 at
1,000 pages measured 245.7 / 249.8 / 255.7 / 259.0 ms across base runs, and
238.5 ms after. Shared mode tracked local within the same spread. **Do not
read the public p50 column as a win.**

## Why contract item 2 is not met, and where the rest is

After the cut the draft is still proportional to total graph pages. The cause
is not in the draft. Measured on an enrolled scratch-backed engine, release,
20 drafts per point:

| | 50 pages | 500 pages |
| --- | ---: | ---: |
| scratch page reads per draft | 34 | 34 |
| scratch bytes read per draft | 320 KB | 2,959 KB |
| largest single scratch page read | 45,432 B | 445,939 B |

The read *count* is constant; a single scratch page grows linearly with the
graph (~890 bytes per page), so every authenticated point read the draft
issues costs bytes proportional to the whole graph. That is
`tine_storage`'s scratch page granularity, reached through
`document_state::load_external_current` / `load_external_current_record` —
the archive-storage and Patricia-index layer the dossier forbids touching.
No change confined to `hot_engine.rs` can make the draft page-proportional
while that holds.

One in-scope adjacent lead was tried and **rejected on measurement**:
`document_dependency_heads` materialized the whole document through
`load_external_current` in order to read the record's `exact_direct_heads`,
even though `load_external_current_record` reads the same authenticated
record without the checkpoint. It removed 11 scratch reads per draft in
debug, but in release the catalog-dependency phase was unchanged
(31.469 ms → 32.105 ms at 1,000 pages, inside noise — the checkpoint is
served warm), while silently under-reporting `document_point_reads` and
`state_page_bytes_read`. Reverted; recorded here as a follow-up that needs
work accounting on the record-only reader before it is worth landing.

## Remaining save-path costs

Unchanged by this cut, from the same receipt at 1,000 pages: the draft is
~27% of the save (63.5 ms of 238.5 ms). `materialize_page_for_projection`
alone is 53.5 ms and carries +47.4 ms of the draft's scaling, of which the
catalog dependency read is +28.2 ms — all of it per-read scratch byte cost,
not redundant work. The report's other three causes (archive accept/index
staging, SQLite apply plus reference-catalog stamp, and the graph-wide
filesystem collision walk) are untouched and remain the majority of the save.

## Decision required from the manager

1. **Widen to the scratch store.** The remaining draft scaling is one
   property: scratch pages grow with the graph, so every point read does.
   Fixing it is a storage-layout change in `tine_storage`, and it would also
   reduce the report's causes 2 and 3, which share the same reader.
2. **Accept the draft as-is.** The catalog copy is gone and the derivation is
   provably page-local; the residue is a storage property that no oplog-level
   cut can reach.
3. **Re-scope.** If the goal is a flat warm save, all four of the report's
   causes plus the scratch page-size property have to go; this cut removes
   about 1.4% of the 1,000-page save's total scaling.

## Focused tests

- `cargo test -p tine-core --lib warm_ every_local_author_transaction_class
  an_ambiguous_logseq_claim in_place_catalog_derivation` → 9 passed
- `RUST_MIN_STACK=33554432 cargo test -p tine-core --lib oplog::` → 1,166
  passed, 2 failed, 19 ignored
- One `cargo fmt --all`; `git diff --check` clean

The two failures — `oplog::import::tests::detached_bootstrap_conflicting_
abandoned_content_address_fails_closed` and `oplog::object_store::
bootstrap_store_tests::detached_publisher_closes_without_changing_ordinary_
index_publication` — were reproduced on `272a3726` with this branch's changes
stashed. They are pre-existing and unrelated; recorded as a follow-up.
`oplog::local_active::tests::a_promoted_archive_with_an_unauthenticated_
resource_claim_is_refused_at_the_state_boundary` overflows the default test
stack in debug on base as well, which is why the suite is run with
`RUST_MIN_STACK`; also a follow-up.

## Limitations

- Local mode drives every conclusion; shared mode tracked local within the
  same spread at both scales.
- 6 edits per configuration. p50 is stable across runs; p95 is not, and no
  conclusion here rests on it.
- Another worktree ran a CPU-saturating build during part of the release
  comparison. The before/after draft-phase deltas are means over the same
  runs and are large relative to that noise; the public p50 column is not,
  which is a second reason not to read it as a win.
- The benchmark harness (`5ab8037e`) and the phase timers were applied
  temporarily and are not in any commit here.
