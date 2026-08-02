# Managed one-page save: measured cause, and the boundary that stops this cut

**Outcome: stopped before changing production code.** The measured cause of
graph-sized work in a warm one-page content save is **not** in the assigned
write set. It is spread across four durable-oplog subsystems, three of which the
dossier explicitly excludes. Fixing only the part that is in scope would remove
17% of the scaling and still leave the save proportional to total graph pages,
so it would not satisfy the frozen contract. This note reports the exact
boundary and the decision the manager now needs to make.

- Tested commit: `272a3726` (worktree HEAD, branch `perf/managed-save-local-reload`)
- Production diff: **none**. Working tree is clean apart from this note.
- Commits: none beyond this note.

## What the dossier assumed, and what is actually true

The dossier measured `save_application_page` and reported "the post-editor
application reload averaged about 155 ms". It also warned: *"do not assume the
visible `reload_application_page` wrapper itself is the cause."* That warning
was correct, and stronger than expected.

The benchmark on `perf/managed-ordinary-latency` times a single phase that spans
both `settle_application_publication` and `reload_application_page`. Splitting
that phase shows the reload is not involved at all:

| per-save mean (release, local mode) | 100 pages | 1,000 pages | Δ |
| --- | ---: | ---: | ---: |
| public `save_application_page` p50 | 68.116 ms | 245.733 ms | +177.6 |
| application translation | 0.607 ms | 0.697 ms | +0.09 |
| `save_editor_page` | 23.095 ms | 98.822 ms | +75.7 |
| `settle_application_publication` | 45.633 ms | 159.561 ms | +113.9 |
| **`reload_application_page`** | **0.455 ms** | **0.465 ms** | **−0.01** |

`reload_application_page` is flat at ~0.46 ms and is 0.2% of the 1,000-page save.
Contract item 2's suggested cut — deleting a repeated reload pass on the return
path — cannot recover any measurable time, and contract item 5's constraint on
the return path is not what is costing anything.

All graph-sized work is inside the durable local-mutation publication, which
`save_editor_page` starts and `settle_application_publication` finishes.

## Where the graph-sized work actually is

Timing the terminal commit pipeline
(`crates/tine-core/src/oplog/operational_coordinator.rs`) by phase:

| coordinator phase | 100 pages | 1,000 pages | Δ | share of scaling |
| --- | ---: | ---: | ---: | ---: |
| bindings | 0.106 ms | 0.108 ms | +0.00 | 0% |
| **draft** | 10.597 ms | 62.659 ms | **+52.06** | 29% |
| capture | 1.043 ms | 1.115 ms | +0.07 | 0% |
| finalize | 0.875 ms | 0.778 ms | −0.10 | 0% |
| archive publish | 0.345 ms | 0.312 ms | −0.03 | 0% |
| **archive stage** | 25.747 ms | 94.052 ms | **+68.31** | 38% |
| **SQLite drain** | 10.526 ms | 44.786 ms | **+34.26** | 19% |
| **projection drain** | 15.105 ms | 47.008 ms | **+31.90** | 18% |
| *(of which: graph-wide collision walk)* | *4.690 ms* | *35.786 ms* | *+31.10* | *17%* |

Four independent causes, each in a different subsystem:

1. **CRDT draft over the whole-graph catalog (29%)** —
   `ShardedHotEngine::draft_admitted_local_author_transaction`
   (`crates/tine-core/src/oplog/hot_engine.rs:11556`) pulls the catalog document
   into the author working set, then `snapshot_engine_documents` /
   `derive_effect_from_snapshots` (`hot_engine.rs:12628`) snapshot and diff it.
   The catalog holds every page, so drafting a one-block edit is O(total pages).
2. **Archive accept/index staging (38%)** — `stage_archive_batch_bounded`
   (`hot_engine.rs:10268`) updates graph-wide keyed structures (page-name
   Patricia index, block-claim index, projection work index). The *count* of
   index writes is constant (33/save at both scales); the *cost per write* grows
   with total graph size.
3. **SQLite apply + reference catalog stamp (19%)** — `tail.drain_ready` at
   `operational_coordinator.rs:592`. Exactly one batch applies per save at both
   scales; the reference-catalog root and coverage digest span all sources.
4. **Graph-wide filesystem collision validation (17%)** —
   `Graph::validate_current_graph_text_collision`
   (`crates/tine-core/src/model.rs:5713`) runs `graph_text_inventory`
   (`model.rs:4846`), a full graph-wide directory walk that `openat`s and
   `fstat`s every managed text file. It is called **three times per durable
   projection write** from `Graph::write_page_projection_with_attempts`
   (`model.rs:15038`, `:15101`, `:15150`).

Cause 4 is the only one inside `model.rs`. Causes 1–3 are in the hot engine, the
archive object store and its Patricia indexes, the tail overlay, and SQLite
materialization — all excluded by the dossier ("Do not touch activation/import,
oplog wire/schema, SQLite schema…") and none of them in the permitted write set.

## Counter proof that nothing else scales

Per-save operation counts are **identical** at 100 and 1,000 pages:

| counter | 100 pages | 1,000 pages |
| --- | ---: | ---: |
| engine document head visits | 1 | 1 |
| engine document point reads | 15 | 15 |
| archive reads | 162 | 162 |
| archive index writes | 33 | 33 |
| SQLite applies | 1 | 1 |
| projection writes | 1 | 1 |
| projection removes | 0 | 0 |
| graph text content reads | 3 | 3 |
| graph text parse attempts | 5 | 5 |
| graph text collision validations | 3 | 3 |
| **graph text inventory entry visits** | **392.5** | **3,092.5** |

Only the inventory walk scales in *count*; everything else scales only in *cost
per operation*. A debug-mode run at 30 and 120 pages pins the walk exactly: 3
collision validations per save at both scales, with entries per walk rising 57 →
147 as pages rise 30 → 120 — exactly one extra entry visited per extra page, per
walk.

## Why no in-scope cut satisfies the contract

Contract item 2 requires the save to be "independent of total graph pages" and
prefers "deleting a repeated inventory/validation/reload pass". The only
repeated pass in scope is cause 4, and it is not accidental redundancy: the
three calls sit at three distinct publication boundaries (before displacement,
after retirement, after publish) and each re-proves the property against
concurrent external writers.

Making cause 4 graph-size-independent needs the graph-wide inode-alias scan
replaced by a point proof. An exactly-equivalent one exists — the resource id is
`hash(dev, ino)` (`model.rs:28102`), so `st_nlink == 1` proves no second
directory entry can alias the inode, and anything else falls back to today's
walk. But landing it requires:

- threading the link count from every identity-capture site (the validator's
  `Option<ContentDigest>` parameter loses it), across ~18 call sites in the
  projection writer — well past "the narrowest directly responsible helper";
- accepting that the fast path no longer re-runs the walk's *incidental* second
  duty, graph-wide inventory-limit and directory-alias refusal, at each
  publication boundary. That is a data-safety/performance tradeoff, not an
  optimization.

And even done perfectly it buys 31.1 ms of 177.6 ms. A 1,000-page save would go
from ~246 ms to ~215 ms against ~68 ms at 100 pages — still plainly proportional
to graph size. The frozen contract would remain unmet.

Per `CONTROLLED-AGENT.md` ("Stop and report when the work requires a new product
decision, architecture, persistent data format, safety/performance tradeoff, or
files outside the permitted write set") and the dossier ("If the measured cause
requires a schema, wire, or architecture change, stop with the exact boundary
rather than inventing one"), this cut stops here.

## Decision required from the manager

Pick one:

- **(A) Widen the write set to the durable oplog.** Causes 1–3 are 81% of the
  scaling and live in `hot_engine.rs`, the archive Patricia indexes, the tail
  overlay, and SQLite materialization. Cause 1 (catalog snapshot/diff per
  author draft) is the most self-contained and is 29% on its own.
- **(B) Authorize the cause-4 safety tradeoff only**, accepting ~13% at 1,000
  pages and an explicitly relaxed contract item 2. Needs a product ruling on
  dropping graph-wide inventory-limit and directory-alias refusal from the
  projection-write fast path.
- **(C) Re-scope the target.** Extrapolating the two points, a save costs
  ~45 ms fixed plus ~0.21 ms per graph page. Even with every graph-sized cause
  removed, a warm one-page save floors at ~45 ms, dominated by durability
  fsyncs. If ~45 ms flat is the goal, all four causes must go.

## Reproduction

No harness is committed (the dossier forbids merging the instrumentation diff
into this lane). To reproduce from `272a3726`:

```
git cherry-pick --no-commit 5ab8037e     # benchmark from perf/managed-ordinary-latency
git apply /tmp/keep-managed-save-instrumentation.patch   # counters + phase timers
cargo build --release -p tine-core --lib --tests
TINE_MANAGED_ORDINARY_SMALL_PAGES=100 \
TINE_MANAGED_ORDINARY_LARGE_PAGES=1000 \
TINE_MANAGED_ORDINARY_BLOCKS_PER_PAGE=10 \
TINE_MANAGED_ORDINARY_EDITS=10 \
cargo test --release -p tine-core --lib \
  managed_ordinary_save_manual_release_receipt -- --ignored --nocapture --test-threads=1
```

The instrumentation added on top of the cherry-pick was:

- `model.rs`: `GraphTextWorkCounters` reading the existing thread-local
  `GRAPH_TEXT_INVENTORY_ENTRY_VISITS` / `GRAPH_TEXT_CONTENT_READS` /
  `GRAPH_TEXT_PARSE_ATTEMPTS` / `GRAPH_TEXT_VALIDATION_TARGET_READS`, plus a
  call counter and an RAII wall-clock accumulator inside
  `validate_current_graph_text_collision_policy`.
- `operational_coordinator.rs`: an RAII per-phase wall-clock accumulator around
  bindings / draft / capture / finalize / archive publish / archive stage /
  SQLite drain / projection drain.
- `sync_runtime.rs`: split the benchmark's single post-editor phase into
  `settle` and `reload`, and surfaced the counters above in the receipt line.

The counters are read inside the actor, because they are thread-locals and the
actor owns its own thread.

## Limitations

- Local mode only in the numbers above; the shared-provider run tracked local
  within noise at both scales (1,000 pages: local p50 255.7 ms, shared 243.4 ms).
- 10 edits per configuration. p50 is stable across the three release runs
  (1,000 pages: 255.8 / 245.7 / 255.7 ms); p95 is noisy (~395 ms) and not used
  for any conclusion here.
- Phase timers are wall-clock inside the actor and sum to slightly more than the
  measured p50 (250.7 ms of phases vs 245.7 ms p50) because p50 is a percentile
  over samples while the phases are means. Shares of scaling are computed from
  the deltas and are not sensitive to this.
- Cause 2 and cause 3 are attributed at phase granularity. The specific
  structure whose per-write cost grows was not isolated further, because doing
  so would only refine work that is already outside the permitted write set.
- No production behavior was changed, so no fail-before/pass-after semantic or
  differential tests were written. Contract items 3–6 are untouched by
  construction.
