# Managed ordinary save: the repeated catalog decode is retained, not re-paid

Branch `perf/managed-save-catalog-cache`, base `fe8e0079`.

**Outcome: the cause the previous lane measured and could not touch is cut.**
The ordinary managed save's SQLite drain drops **71%** at 1,000 pages
(44.8 ms → 12.9 ms) and the drain's per-graph-page slope drops **81%**
(0.0397 → 0.0076 ms/page). The whole public `save_application_page` p50 drops
**16.7%** at 1,000 pages in local mode. Nothing about the accepted oplog,
archive, schema, or any query surface changes.

| Commit | Subject |
| --- | --- |
| `6ef969a2` | `perf(oplog): reuse the accepted catalog decode across ordinary saves` |
| `b05d0503` | `test(oplog): bound the ordinary save's catalog decode to catalog change` |

Diff `fe8e0079..b05d0503`: 2 files, +723 / −4. No schema, DDL, wire, version
constant, durable format, archive object, encoding, index, or public query
surface changed.

## What was repeated, and what now is not

The previous lane's note (`managed-save-sqlite-drain-opus.md`) ended by
measuring, not assuming, the drain's scaling: 92% of it was one decode of the
whole page-catalog CRDT document per save, inside `ShardedHotEngine`, and it
recommended a reuse keyed on `(document_id, causal_state_digest)`. That
recommendation is what this lane implements. Its own numbers are reproduced and
confirmed below.

`AcceptedRootMaterializer::materialize_page` resolves the edited page's home
document through the catalog. That resolution called
`reconstruct_projection_frontier` → `load_external_exact`, which reads the
catalog's whole exact-state checkpoint out of scratch, SHA-256s it, and imports
it through Loro. All three are O(total graph pages), to learn one
`page_id -> home_document_id` pair. A content-only edit does not touch the
catalog, so nine of ten consecutive saves decoded a byte-identical document the
same process had just decoded.

`ShardedHotEngine` now retains exactly one decoded catalog
(`hot_engine.rs:17116` / `:17160`), keyed by the content identity that already
determines its bytes: `(document_id, causal_state_digest)` — the same pair the
exact-state scratch lane keys the encoded checkpoint by. `load_document`
(`hot_engine.rs:4458`) consults it only for the catalog document, and only after
the authority step has already run.

## Why reusing the decode grants nothing

The retained entry carries no authority; it is a decode, not a decision.

- **The authority runs first, on every resolution, hit or miss.**
  `accepted_frontier_document(&self.root, document_id)` is what proves *which*
  causal state this accepted root selects for this document, and it refuses a
  blocked engine, an unauthenticated root and an absent document before any
  reuse is considered (`hot_engine.rs:4462`). A hit is only ever allowed to skip
  re-decoding bytes that the proved `causal_state_digest` already names.
- **A catalog mutation misses.** A rename, create, delete or path move changes
  the catalog's causal state, so the key does not match and the full decode
  runs.
- **A different or historical root misses** for the same reason — its
  `accepted_frontier_document` yields a different causal state.
- **Publication happens only after success.** `retain_accepted_document` is
  called only after the full load and every integrity proof inside it succeeded
  (`hot_engine.rs:4502`), so a refused or malformed checkpoint returns earlier
  and can never replace a valid entry.
- **The Loro handle is re-proved before every reuse.** A `LoroDoc` is
  reference-counted with interior mutability, so the entry records its Loro
  version identity and re-checks it on each hit; anything that advanced or
  checked the document out behind the engine's back is refused and decoded again
  rather than served (`hot_engine.rs:17133`).
- **The entry never outlives its store.** It is dropped whenever the scratch
  store it was resolved against is replaced — refused-resume scratch
  re-creation (`hot_engine.rs:6989`) and snapshot recovery
  (`hot_engine.rs:7611`).
- **Only the catalog is retained.** Membership and home shards are point-sized;
  caching them would buy a fixed cost and pay for it with a resident set that
  grows with the pages an event touches.

Accounting was split so the two are never conflated: `exact_catalog_decodes`
sits beside the existing `exact_catalog_loads`, so "resolved the catalog" and
"read the whole catalog checkpoint again" are separate numbers, and the
fresh-candidate structural accounting invariant in `SqliteFrontier` gained
`decodes <= loads`.

## Before / after receipts

Same machine, idle (load average 0.03), **same release binary**, two
back-to-back runs of 19m08s and 19m06s. The before column is that binary with
`TINE_MANAGED_ORDINARY_UNCACHED_CATALOG=1`, which selects the pre-cut path via
`set_retained_catalog_enabled_for_test(false)` — it disables both reuse *and*
retention, so every catalog resolution takes the full decode. The receipt's own
counters confirm the switch worked rather than being assumed: before is
`1.00` decodes and `0.00` reuses per save, after is `0.10` and `0.90`.

10 warm one-page content edits on an existing page, 10 blocks per page, both
modes, per-save means unless marked.

### 1,000 pages

| per save | local before | local after | Δ | shared before | shared after | Δ |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| public `save_application_page` p50 | 255.086 ms | **212.515 ms** | **−16.7%** | 232.353 ms | **208.452 ms** | **−10.3%** |
| public p95 | 384.597 ms | 377.535 ms | −1.8% | 370.466 ms | 374.760 ms | +1.2% |
| actor total | 269.531 ms | 229.852 ms | −14.7% | 251.239 ms | 229.175 ms | −8.8% |
| — editor save (CRDT draft + publish) | 103.295 ms | 99.773 ms | −3.4% | 94.119 ms | 98.213 ms | +4.4% |
| — application reload | 165.777 ms | 129.526 ms | −21.9% | 156.677 ms | 130.418 ms | −16.8% |
| **SQLite drain** | **44.822 ms** | **12.886 ms** | **−71.3%** | **43.294 ms** | **12.345 ms** | **−71.5%** |
| **catalog decode** | **34.083 ms** | **3.608 ms** | **−89.4%** | **34.861 ms** | **3.416 ms** | **−90.2%** |
| catalog decodes | 1.00 | 0.10 | | 1.00 | 0.10 | |
| catalog reuses | 0.00 | 0.90 | | 0.00 | 0.90 | |

### 100 pages

| per save | local before | local after | Δ | shared before | shared after | Δ |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| public `save_application_page` p50 | 61.937 ms | 58.926 ms | −4.9% | 66.484 ms | 63.173 ms | −5.0% |
| public p95 | 92.134 ms | 92.969 ms | +0.9% | 89.155 ms | 91.445 ms | +2.6% |
| actor total | 66.234 ms | 63.348 ms | −4.4% | 68.035 ms | 65.456 ms | −3.8% |
| — editor save | 22.282 ms | 22.322 ms | +0.2% | 22.576 ms | 22.906 ms | +1.5% |
| — application reload | 43.257 ms | 40.501 ms | −6.4% | 45.025 ms | 42.037 ms | −6.6% |
| **SQLite drain** | **9.080 ms** | **6.059 ms** | **−33.3%** | **9.414 ms** | **6.251 ms** | **−33.6%** |
| **catalog decode** | **3.527 ms** | **0.356 ms** | **−89.9%** | **3.555 ms** | **0.350 ms** | **−90.2%** |
| catalog decodes | 1.00 | 0.10 | | 1.00 | 0.10 | |
| catalog reuses | 0.00 | 0.90 | | 0.00 | 0.90 | |

### The before column is independently corroborated

The pre-cut numbers this switch produces match what the previous lane measured
on the committed pre-cut code with different instrumentation, at the same scale
and configuration: drain 44.822 ms here vs 44.873 ms there, catalog load
34.083 ms here vs 34.515 ms there. The switch reproduces the real prior path,
not some other one.

## Where the win comes from, and where it does not

**The cut removes the decode and only the decode.** Subtracting the measured
catalog decode from the drain leaves the rest of the drain unchanged:

| local, drain minus catalog decode | 100 pages | 1,000 pages |
| --- | ---: | ---: |
| before | 5.553 ms | 10.739 ms |
| after | 5.703 ms | 9.278 ms |

**Scaling** (local, slope across 100 → 1,000 pages):

| per graph page | before | after | |
| --- | ---: | ---: | ---: |
| SQLite drain | 0.0397 ms | **0.0076 ms** | −81% |
| catalog decode | 0.0340 ms | **0.0036 ms** | −89% |
| actor total | 0.2259 ms | 0.1850 ms | −18% |

The catalog-decode slope falls by almost exactly the decode-count ratio
(0.106× vs 0.10×): what remains is one cold decode amortised over ten saves,
which is the floor for this design.

**Counts confirm the timers.** Work that should not have moved did not, and the
one count that should have moved did:

| per 10 saves, local | 100 before | 100 after | 1,000 before | 1,000 after |
| --- | ---: | ---: | ---: | ---: |
| SQLite applies | 10 | 10 | 10 | 10 |
| projection writes | 10 | 10 | 10 | 10 |
| archive index writes | 330 | 330 | 330 | 330 |
| archive reads | 1602 | 1603 | 1627 | 1606 |
| document head visits | 10 | 10 | 10 | 10 |
| **document point reads** | **140** | **131** | **140** | **131** |

The nine fewer point reads per ten saves are exactly the nine reused decodes not
re-reading the catalog's exact-state checkpoint out of scratch. This is a
timer-free confirmation of the same fact.

**What this does not improve, stated plainly:**

- **The tail is unchanged.** With 10 samples the reported p95 *is* the slowest
  single save, and the after path still pays exactly one full catalog decode per
  run (0.10 × 10 = 1). The retained decode cannot help the save that misses, so
  p95 moving ±2% is noise, not a regression.
- **The CRDT draft phase is untouched.** `editor save` moves −3.4% local /
  +4.4% shared at 1,000 pages, i.e. run-to-run noise in both directions. That
  path reads the unchanged catalog in place already (`f0a51795`, the previous
  one-page-draft lane) and does not go through this materializer.
- **At 100 pages the whole-save win is ~5%.** The decode is only graph-sized;
  at small graphs it is not what the save is spending its time on.

## Fail-before / pass-after

The four proofs and their fail-before behaviour were established when
`b05d0503` was authored; that work is not redone here. What this session
verified is that they pass on the pushed tree with the temporary probes removed.

| Test | Result |
| --- | --- |
| `ordinary_content_saves_decode_the_unchanged_catalog_once` | pass |
| `a_changed_catalog_is_decoded_once_and_never_confused_with_another_root` | pass |
| `a_mutated_retained_catalog_is_refused_and_decoded_again` | pass |
| `ordinary_drained_saves_match_clean_archive_replay_across_rich_shapes` | pass |
| `ordinary_drain_reconstructs_each_accepted_event_once` (prior lane) | pass |
| `ordinary_applies_count_the_reference_coverage_table_once_per_open` (prior lane) | pass |

Two of these are standing, not one-shot, checks:

- `ordinary_content_saves_decode_the_unchanged_catalog_once` asserts the
  accounting is *identical* at 1 page and at 41 — the "cost your own delta, not
  the graph" property stated as an exact identity, so it fails if the decode
  ever returns.
- `ordinary_drained_saves_match_clean_archive_replay_across_rich_shapes` runs its
  whole rich program **twice**, once with the retained decode disabled, which
  keeps the pre-cut path in the tree as a permanent differential oracle. Both
  runs are compared against a clean archive replay and against their own reopen,
  and must additionally publish identically to each other: same frontier root,
  semantic projection digest, authenticated reference catalog root, per-table row
  digests, and every public query surface. It also asserts the retained run
  actually reused a decode, so the two runs cannot agree vacuously.

## Commands and results

| Command | Result |
| --- | --- |
| `cargo test --release -p tine-core --lib -- oplog::sqlite::tests::ordinary_ a_changed_catalog_is_decoded_once_and_never_confused_with_another_root a_mutated_retained_catalog_is_refused_and_decoded_again` | 6 passed, 0 failed |
| `cargo fmt --all -- --check` | clean |
| `git diff --check` | clean |

`RUST_MIN_STACK=134217728` is set for the `tine-core` runs, as in the preceding
notes.

The receipt runs were:

```
RUST_MIN_STACK=134217728 \
TINE_MANAGED_ORDINARY_SMALL_PAGES=100 TINE_MANAGED_ORDINARY_LARGE_PAGES=1000 \
TINE_MANAGED_ORDINARY_BLOCKS_PER_PAGE=10 TINE_MANAGED_ORDINARY_EDITS=10 \
[TINE_MANAGED_ORDINARY_UNCACHED_CATALOG=1] \
cargo test --release -p tine-core --lib \
  managed_ordinary_save_manual_release_receipt -- --ignored --nocapture
```

The benchmark itself asserts semantics on every one of the 10 edits in both
modes: the returned DTO matches the on-disk parser's view of the projected page,
the runtime stays `Active`, the save is `Saved` (never deferred), and after a
clean shutdown and reopen the last edit is still present — so these are timed
runs of a correct save, not of a fast wrong one.

## Preserved invariants

- The accepted oplog event, prefix, frontier, and every durable archive object
  are byte-identical. The cut authors, encodes and stores nothing; it reuses an
  in-memory decode of bytes already named by a proved causal-state digest.
- SQLite schema, schema version, oplog wire format, archive format, provider
  protocol and projection format unchanged.
- Every public query surface is proved semantically identical against clean
  archive replay *and* against the pre-cut path, per table and per query,
  including FTS.
- Root authority is unchanged and is re-proved per resolution:
  `accepted_frontier_document` runs on every hit and every miss. Re-materialising
  a historical root while the retained decode holds a later catalog is explicitly
  tested and decodes its own catalog.
- Restart, cold reopen, archive replay, rollback/refusal, interruption recovery,
  shared-provider mode and external reconciliation unchanged. Shared mode is
  measured in the receipt above and tracks local within noise.
- Durability unchanged: same transaction, same candidate proofs, same WAL
  checkpoint, same projection-checkpoint write, same base-revision gating. The
  save is still durably committed before success is published. The receipt's
  apply, projection-write and archive-index-write counts are identical before and
  after.
- Fallback and refusal remain available at every point: an absent entry, a key
  mismatch, a Loro version mismatch, a replaced scratch store, or a non-catalog
  document all take the existing full-load path.
- Memory is bounded by construction: exactly one retained document per engine,
  and an engine owns one workspace.
- The terminal SQLite construction, its structural accounting invariant (now
  including `decodes <= loads`), the window-bounded catalog authority from
  `982a3ef4`, and both cuts from `fe8e0079` are unchanged and still asserted.

## Limitations and follow-ups

1. **The instrumentation is not committed.** The catalog decode/reuse probe, the
   `drain_ready` timer, the actor-side per-phase receipt and the
   `managed_ordinary_save_manual_release_receipt` benchmark were temporary and
   were removed before this note. To reproduce the tables above from `b05d0503`:
   re-add the benchmark and actor receipt in `sync_runtime.rs`, a thread-local
   nanosecond/decode/reuse probe around `reconstruct_projection_frontier` in
   `AcceptedRootMaterializer::load_document`, and a wall-clock wrapper around
   `TailOverlay::drain_ready`. Counters are read inside the actor because they are
   thread-locals and the actor owns its thread. The `_UNCACHED_CATALOG` switch is
   a one-line call to the committed
   `set_retained_catalog_enabled_for_test(false)`.
2. **`application_reload` is now the largest remaining cost of the save** —
   129.5 ms of a 229.9 ms actor total at 1,000 pages, of which the drain is only
   12.9 ms. The other ~117 ms is not attributed by this lane's instrumentation.
   *Follow-up: it is now the obvious next target.*
3. **`editor save` is ~100 ms and did not move.** The previous draft lane cut its
   catalog read; whatever remains is a separate cost and was not measured here.
   *Follow-up.*
4. **One cold decode per process remains** and lands in the tail. Retaining
   across the runtime's lifetime already covers it after the first save; nothing
   cheaper exists without pre-warming, which would move the cost rather than
   remove it.
5. **`write_projection_checkpoint`** (~1.6 ms per save, flat) is still the
   largest fixed item inside the drain, unchanged and still not graph-sized.
   *Follow-up, carried over from the previous note.*
6. **10 samples per configuration.** Enough for the p50 and for the means, which
   is what every claim here rests on; not enough to characterise the tail. No
   claim above rests on p95.
