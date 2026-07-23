# Perf bench — spotting regressions

Tine exists to be fast at the thing OG Logseq is slow at: **loading and scrolling
a large page on a modest machine.** `npm run bench` measures that objectively and
repeatably, so a refactor that quietly makes it slower gets caught instead of
shipped. It is a **regression detector, not a profiler** — it answers "did this
get meaningfully worse?", not "how many microseconds does X take."

```bash
source scripts/env.sh
npm run build            # the bench serves dist/ via `vite preview` (a PROD build)
npm run bench            # measure + compare to scripts/bench-baseline.json
npm run bench -- --update  # re-record the baseline (do this on a QUIET machine)
npm run bench -- --update --output /tmp/tine-bench.json  # record without changing the local baseline
```

Run the node script directly — **no `timeout` wrapper** (it would orphan the vite
child; the script SIGKILLs vite itself in a `finally`).

## What it measures

Headless Chromium drives the mock backend's gated **2000-block "Big" page**
(`?big` in `src/mock.ts`). A local invocation times each metric **in-page** with
`performance.now` and reports the **min of K=8** runs (least noise), after one
discarded warmup. Manual full/focused CI wraps the same harness in the
multi-round protocol below.
It boots once and measures **warm navigations** (journals ↔ Big) so the numbers
reflect the app's own mount/render cost, not per-reload JIT + WASM-compile jitter.

| metric      | what | character |
|-------------|------|-----------|
| `bigLoad`   | switch to the 2000-block page → stable render | **coarse** — mounting 2000 Solid components is GC-sensitive; ~10–15% run-to-run noise, more on a loaded machine. Catches gross regressions (a doubling), not micro-drift. |
| `scrollBig` | scroll the Big page to the bottom → settle (blocks render on demand) | tight (~a few %). |
| `parseStats`| `window.__tineParseStats` after a cold Big open — cold parses ≈ blocks actually parsed | exact. **~12 on a 2000-block page = block virtualization is intact.** If this jumps toward 2000, lazy-body rendering broke. |

`parseStats` works in the prod build because the bench sets `window.__tineBench`
before boot (see the `statsEnabled()` opt-in in `src/render/parse.ts`); for normal
users the counter is dead-code-eliminated.

## Tier 2 — the calibration normalizer ("code, or machine?")

A fixed, deterministic CPU loop (`calib` ms) measures this machine's cost for a
unit of work right now. Every app metric is reported **raw** and **normalized =
raw / calib**, and the baseline stores the normalized numbers — so a baseline
recorded on a fast box still roughly holds on a slow one.

- If `calib` is **> 1.5× its baseline**, the machine is throttled/loaded → the run
  prints "UNRELIABLE, re-run cooler" and does **not** fail.
- Otherwise each normalized metric is compared to the baseline; anything more than
  **30%** worse is flagged `REGRESSED` and the run exits non-zero (so this can gate
  CI later). The 30% sits above `bigLoad`'s noise floor on purpose.

The local baseline (`scripts/bench-baseline.json`) remains useful for quick
developer checks. CI does not compare a candidate on one machine to that file's
numbers from another machine: the calibration loop proved too different from
browser layout/paint to normalize scrolling reliably. Accordingly, `npm run
bench` reports a different-machine baseline as advisory and does not fail; a
baseline recorded on the same machine retains the local regression exit code.

## Release/focused CI hard gate: same-machine A/B without baseline ratcheting

The manual performance job checks out and builds three exact trees on one
runner: the candidate, the immutable long-term anchor, and the most recently
published release. It runs
three interleaved rounds and rotates the order, so every version occupies the
first, middle, and last position once. Each decision uses the **median of the
three round minima**, not a single invocation. The artifact retains every round,
its exact order, and its within-round samples.

Reliability is checked at both levels: calibration spread across all nine
measurements, and per-version/per-metric spread across rounds. A volatile
`scrollBig` baseline therefore fails as unreliable even when its median would
make the candidate pass. Rerunning a red job cannot erase the first attempt's
evidence; every attempt uploads its complete distribution.

`scripts/bench-policy.json` is the contract:

- `v0.4.7` is the immutable pre-Sheets/pre-split-view anchor. It does not advance
  merely because a slower release shipped, so repeated small regressions cannot
  ratchet the allowed performance downward.
- `previousRelease.ref` advances during release preparation to the latest already
  published version. It catches a large one-release jump.
- Budgets are metric-specific: large-page load is allowed 25% over the immutable
  anchor and 15% over the previous release; scroll is allowed 30% and 20% because
  its browser scheduling noise differs. Round-spread limits are 30% for the
  coarser load metric and 15% for scroll. Cold parse misses use the worst of the
  three rounds and have an absolute cap of 15, independent of timing.

The job is a hard release gate when `ci.yml` is dispatched with `scope=full`.
Use the same workflow with `scope=performance` for focused proof between
releases; it does not count as full release evidence. A feature expected to
exceed a budget stops for a product decision and performance design; do not move
either baseline to make it pass.

## Native startup and early-frame paint

The Chromium benchmark does not include Tauri process creation, WebKit startup,
session restoration, or the first native frame. Before a release, build the
production-protocol binary and run:

```bash
source scripts/env.sh
npm run bench:startup
```

This downloads and caches the published Linux v0.4.7 binary under ignored
`test-results/` (never under the home directory), then launches v0.4.7 and the
candidate in alternating order. Every sample gets fresh XDG state and a private
D-Bus session so Tine's single-instance forwarding cannot contaminate the
measurement. The graph is deterministic: 80 pages derived from the public
kitchen-sink fixture plus a 120-block journal.

The report records native session creation and first visible journal content,
using eight measured samples per binary. Any median or p95 more than 30% slower
than the immutable v0.4.7 anchor fails. Separate, unmeasured visual trials retain
frames at session attach, first content, +100 ms, +300 ms, and +1 s; inspect them
for new blank/intermediate/corrupt paints even when the final timing is within
budget. Override binaries or output with `TINE_STARTUP_BASELINE`,
`TINE_STARTUP_CANDIDATE`, and `TINE_STARTUP_ARTIFACT_DIR`.

## Graph-scale bench (Rust core, on-disk graph)

`npm run bench` drives the **mock** backend, so it can only see frontend
render/scroll cost — never graph-scale core scans (page lookups, queries, publish,
the switcher). Those live in `tine-core` over a real directory graph, so they have
their own harness: the `graph_scale_bench` example.

```bash
source scripts/env.sh
cargo run --release --example graph_scale_bench -p tine-core -- 2000 10000
```

It generates a synthetic graph (N pages incl. ~15% **nested** under `pages/ns-*/`,
30 journals, and a query-heavy `Dashboard.md` of `{{query}}`/`{{embed}}` macros)
and prints a per-scale table: `cold_open`, `cache_build`, **`find_entry_K`**
(500 distinct `load_named` lookups — the page-lookup fanout), `switcher`,
`warm_query`, and **`publish`** (runs the Dashboard's queries/embeds during export).
Scale = page count; args override the default `2000 10000 20000`. Output graphs go
to `/tmp/graph-scale-bench-<scale>` (regenerated each run, seeded/deterministic).

Use it to get a before/after curve on the graph-scale perf-audit fixes: a healthy
scan is ~flat per-item as scale grows; a fanout (e.g. `find_entry` re-walking the
dir on every lookup) grows with scale.

There is also `sheets_phase0_bench` (query/edit-cycle costs at 10k–200k blocks) in
the same examples dir.

## Sparse-oplog sharding and ownership format gate (2026-07-22)

`oplog_sharding_bench` is a deterministic standalone Loro layout harness, not a v2
engine or SQLite benchmark. Its parent starts one fresh child per requested layout
and waits for it before starting the next. Linux current/peak RSS comes from
`VmRSS`/`VmHWM`; peak columns are cumulative child peaks at the sample point, not
phase-isolated peaks. Other platforms print `unsupported`.

Each child writes below the deterministic PID path
`/tmp/tine-oplog-sharding-<pid>-<layout>-<pages>-<blocks>`. A drop guard removes the
path after success and ordinary Rust error unwinding, and the parent also removes
the known child path after a nonzero exit. Before starting a layout, the next run
removes inactive stale benchmark PID paths. SIGKILL, and signals that terminate
without Rust unwinding, can leave the path until that next run; the harness makes no
stronger interrupt-cleanup claim.

At 10,000 pages/1,000,000 blocks there are 100 blocks per immutable home shard.
`home-owner` leaves every block on its home page. `home-fanout` keeps the entity and
owner register in that same immutable home shard but deterministically assigns
`current_page = (home_page + 37 * home_order) mod page_count`. Thus one current page
has 100 members from 100 distinct home shards at 1M.

The corrected home materializer reads current-page membership claims, parses and
deduplicates their home shard IDs, loads each referenced home shard, checks the
authoritative owner register, extracts content only for a matching live claim, and
drops that home document before loading the next. It does not equate current page
with home page. The 1k-page case retains only extracted block strings and claim
metadata; it does not retain non-editing Loro home documents.

The convergence probe uses four Loro documents: one stable entity/owner home and
three source/destination membership documents. Every move or delete is exported as
the complete set of changed update objects. It applies whole batch sets in A→B and
B→A order and compares canonical materialized state only after both complete
batches. It runs and asserts exactly these cases:

- move/move selects one owner, retains both destination claims as CRDT evidence, and
  hides the losing claim;
- move/edit retains the edit at the winning destination;
- move/delete converges with the tombstone winner invisible;
- delete/edit retains recoverable home content but materializes no tombstoned block.

The rename probe is explicitly a synthetic affected-only lower bound. It updates the
catalog page path and a configured deterministic set of referrer page shards that a
future reverse index is assumed to have selected. It measures only Loro mutation and
incremental update export. It neither performs nor measures SQLite lookup,
authorization, semantic validation, object envelopes, or durable batch commit.

### Machine and exact commands

Measured at commit `a50fea8c` on Linux 6.1.0-45-amd64, AMD Ryzen 5 8600G (6 cores,
12 threads), 30 GiB RAM, no swap. Before the full runs, `free -h` reported 11 GiB
available; the overlay containing `/tmp` had 57 GiB available and was 94% used.
Other long-running development processes were present, CPU frequency scaling was
enabled, and this was not an otherwise idle lab host. Treat wall time as a local
format decision receipt with generous ceilings, not as a portable absolute speed
claim. RSS is the stronger layout discriminator.

```bash
rtk uname -a
rtk lscpu
rtk free -h
rtk df -h /tmp
rtk cargo check -p tine-core --example oplog_sharding_bench
rtk cargo run --release -p tine-core --example oplog_sharding_bench -- --pages 20 --blocks 2000 --layout all --rename-referrers 8
rtk cargo run --release -p tine-core --example oplog_sharding_bench -- --pages 10000 --blocks 1000000 --layout home-owner --rename-referrers 32
rtk cargo run --release -p tine-core --example oplog_sharding_bench -- --pages 10000 --blocks 1000000 --layout home-fanout --rename-referrers 32
rtk cargo run --release -p tine-core --example oplog_sharding_bench -- --pages 10000 --blocks 1000000 --layout catalog-owner
rtk cargo run --release -p tine-core --example oplog_sharding_bench -- --pages 5000 --blocks 500000 --layout graph-wide
rtk cargo run --release -p tine-core --example oplog_sharding_bench -- --pages 7500 --blocks 750000 --layout graph-wide
```

The catalog and graph-wide commands are the earlier control receipts. The correction
did not rerun catalog at 1M because its representation did not change, and it did not
rerun unsafe graph-wide at 1M. It also did not rerun 750k graph-wide because no
graph-wide schema changed.

For completeness, the exact historical command that was interrupted was the
following; it is recorded as operator evidence and was **not** rerun in this
correction:

```bash
rtk cargo run --release -p tine-core --example oplog_sharding_bench -- --pages 10000 --blocks 1000000 --layout graph-wide
```

### Raw measurements

Times are single-run optimized wall times on the machine above. Bytes and KiB are raw
harness integers. Corrected 2k smoke and 1M home rows follow; the catalog/graph-wide
control rows are retained from the prior run and have no fanout/rename columns.

| scale/layout | objects | build ms | encode ms | object write ms | encoded bytes |
|---|---:|---:|---:|---:|---:|
| 2k / graph-wide | 1 | 18.326 | 4.451 | 0.381 | 335,605 |
| 2k / catalog-owner | 21 | 10.903 | 3.280 | 0.977 | 381,552 |
| 2k / home-owner | 21 | 14.466 | 4.306 | 1.235 | 428,268 |
| 2k / home-fanout | 21 | 13.255 | 3.945 | 1.146 | 428,268 |
| 1M / catalog-owner | 10,001 | 6,254.729 | 1,574.619 | 550.857 | 192,608,897 |
| 1M / home-owner | 10,001 | 5,993.216 | 1,656.850 | 585.081 | 215,959,441 |
| 1M / home-fanout | 10,001 | 5,941.066 | 1,653.381 | 588.445 | 215,959,441 |
| 750k / graph-wide | 1 | 6,862.655 | 2,361.905 | 78.958 | 129,562,173 |

| scale/layout | cold ms / current/peak KiB | fanout / homes / blocks | 1k materialize ms / current/peak KiB | homes / blocks |
|---|---:|---:|---:|---:|
| 2k / home-owner | 2.129 / 5,732/5,788 | 1 / 1 / 100 | 37.129 / 7,792/7,892 | 20 / 2,000 |
| 2k / home-fanout | 11.783 / 5,832/5,884 | 20 / 20 / 100 | 37.317 / 7,904/7,904 | 20 / 2,000 |
| 1M / home-owner | 13.368 / 14,240/14,240 | 1 / 1 / 100 | 1,990.415 / 25,224/25,224 | 1,000 / 100,000 |
| **1M / home-fanout** | **64.306 / 14,240/14,240** | **100 / 100 / 100** | **3,825.617 / 25,260/25,260** | **4,663 / 100,000** |

| scale/layout | edit ms / update B | move ms / update B | rename referrers / affected docs / ms / update B | convergence |
|---|---:|---:|---:|---|
| 2k / catalog-owner | 0.101 / 131 | 0.048 / 518 | 8 / 9 / 0.056 / 1,418 | owner LWW only |
| 2k / home-owner | 0.098 / 131 | 0.019 / 277 | 8 / 9 / 0.050 / 1,416 | four complete-batch cases pass |
| 2k / home-fanout | 0.096 / 131 | 0.019 / 277 | 8 / 9 / 0.066 / 1,416 | four complete-batch cases pass |
| 1M / home-owner | 0.104 / 131 | 0.019 / 277 | 32 / 33 / 0.369 / 5,259 | four complete-batch cases pass |
| 1M / home-fanout | 0.124 / 131 | 0.021 / 277 | 32 / 33 / 0.382 / 5,259 | four complete-batch cases pass |

`sequential_read_ms` and `sequential_one_doc_import_ms` are exactly a sequential
one-object-at-a-time file read and `LoroDoc::import`; each document is immediately
dropped. They are not operation-batch replay, semantic-effect/frontier validation,
materialized-state accumulation, SQLite rebuild, or phase-isolated peak RSS.

| scale/layout | sequential read ms | sequential one-doc Loro import ms | final current / child peak KiB |
|---|---:|---:|---:|
| 1M / home-owner | 228.110 | 4,257.889 | 23,072 / 25,224 |
| 1M / home-fanout | 226.018 | 4,213.611 | 23,540 / 25,260 |

There is deliberately no completed 1M graph-wide result. During the earlier unsafe
attempt, an operator observed 10,400,100 KiB child RSS at about 50 seconds and 1.9
GiB host memory available with no swap, then sent SIGINT before encode/load. Those
numbers came from external operator sampling, not harness output. The exact sampling
commands were:

```bash
rtk ps -C oplog_sharding_bench -o pid,ppid,etime,rss,vsz,cmd
rtk free -h
```

The absence of a leftover directory after that particular interrupt was also an
operator observation, not a signal-cleanup guarantee. The largest completed
graph-wide run remains 750k at 7,952,800 KiB child peak. No 1M number is extrapolated.

The `catalog-owner` layout is also rejected. Its small encoded representation does
not compensate for a 621,304 KiB catalog-plus-page cold open, because every open or
ownership authorization loads the million-entry owner map. It also separates moving
content from the concurrent ownership register, so a timing win would not by itself
settle concurrent move/edit semantics.

### Accepted layout and distinct evidence gates

The intended v2 shape remains catalog + stable home page shards. The sole live
`PageId -> path` is the catalog page-state register. The sole live
`BlockId -> PageId|Tombstone` and entity-to-shard mapping is in the block's stable
home shard; page membership carries that home-shard ID. Losing membership claims are
filtered and never materialized. Mergeable block content remains in the same home
shard, so it is not copied between CRDT documents on a move.

The logical causal frontier is `FrontierV2`, a canonical `DocumentId`-sorted list.
Empty document entries are omitted; a globally empty frontier is valid for a true
empty baseline. Every non-empty entry contains canonical sorted unique
`(CrdtPeerId, max_counter)` entries plus the exact direct `BatchId` heads relevant
to that document. A relevant head updates the document and has no relevant
descendant in the declared atomic-batch DAG, including descendant paths through
batches that update other documents. The causal-state digest binds the document,
counters, and exact direct heads. Authenticated exact document-state and causal-clock
indexes reconstruct the declared dependency state without manifest-ancestry scans;
they exclude concurrent non-dependencies, apply only the manifest's closed update-
object set, and hash the canonical affected-entity delta. Unavailable frontiers
stage; redundant ancestors, unrelated maxima, omitted heads, and digest mismatches
reject before visibility. ADR 0049 contains the complete manifest/object,
projection, import, and fencing contract, including an explicit
`projection_schema_version` separate from receipt-format, projection-policy, and
managed-entity-set versions.

The representative fanout row, not the home-local row, selects the v2 architecture.
Its P0A CRDT recovery/fallback evidence is deliberately separate from the later
SQLite-backed normal startup/hot-state activation evidence.

| gate | ceiling | evidence | result |
|---|---:|---:|---|
| cold one-page claimed-content materialization (100 home shards) | <= 1,000 ms and <= 262,144 KiB current RSS | 64.306 ms / 14,240 KiB | PASS |
| P0A CRDT recovery/fallback materialization (1,000 pages, 100,000 blocks, 4,663 home shards) | <= 5,000 ms and <= 524,288 KiB current RSS | 3,825.617 ms / 25,260 KiB | PASS |
| P2.2 normal user-facing startup/hot state (1,000 pages, 100,000 blocks) | <= 2,000 ms and <= 524,288 KiB current RSS | not implemented by this Loro harness; requires SQLite-backed startup | **UNPROVEN until P2.2** |
| one-page edit and update | <= 2 ms and <= 4,096 bytes | 0.124 ms / 131 B | PASS lower bound |
| cross-page move/owner update | <= 5 ms and <= 16,384 bytes | 0.021 ms / 277 B | PASS lower bound |
| authenticated full operation-batch replay (offline recovery) | <= 45,000 ms and <= 1,048,576 KiB peak RSS | 39,832.674 ms / 223,812 KiB at P1A.2 | PASS |
| bounded startup tail replay | P2.2 must set/meet the numeric sub-gate within the normal startup ceiling | not implemented by this harness | **UNPROVEN until P2.2** |
| complete SQLite rebuild to canonical state | <= 60,000 ms and <= 1,048,576 KiB peak RSS | not implemented by this harness; the budget includes authenticated replay plus projection work | **UNPROVEN until P2** |
| deterministic fixture build+encode+write | <= 15,000 ms and <= 402,653,184 encoded bytes | 8,182.892 ms / 215,959,441 B | PASS fixture-only |

The adversarial fanout result accepts stable immutable-home plus current-membership
for P0A recovery/fallback materialization. That recovery work is bounded by open
content and the distinct home shards it references, not by total graph residency.
It does not pass or fail the separate normal startup gate: this harness has no
SQLite materialization, canonical rebuild, bounded tail replay, or normal startup
path. `LocalActive` remains disabled until the P2.2 startup/hot-state gate and the
other implementation receipts pass. ADR 0049 acceptance records the architecture
and logical format choice; it does not mean production activation.

The P1A.2 full-replay receipt is a separate, fresh-process recovery measurement over
25 authenticated batches containing exactly 1,000,000 blocks and 10,000 page
operations. It validates immutable manifests and update objects, reconstructs exact
causal and semantic state, and publishes authenticated current and exact document
checkpoints. The optimized runs took 37.860 and 39.833 seconds; the slower run peaked
at 223,812 KiB. The 45-second ceiling preserves a measured regression margin. It is
not a normal startup target and does not relax the P2.2 2-second SQLite-backed
startup/hot-state gate.

The same receipt retained zero owned semantic-snapshot entries for the million new
blocks, reduced external-history page reads from 70,996 to 988, and materialized one
100-block page in 39.180 ms with exactly one manifest and one object read. Remaining
full-replay time is dominated by canonical per-document CRDT tracker sealing. Further
improvement requires an aggregate persistent representation rather than another
local-path tweak, so that format-level redesign is deferred unless later P2 evidence
shows it is needed for user-facing startup or bounded local operations.

The **16 MiB retained staged-object / 10,000 unapplied-batch** thresholds are only a
conservative provisional safety cap, whichever comes first. This harness does not
create real envelopes, retain a tail, run an applier, or exercise backpressure, so it
does not measure or pass the cap. P1 must measure real envelope sizes, retained-tail
memory, applier behavior, and visible backpressure before format freeze or
activation; P2 must prove frontier-transaction and rebuild behavior.

## Deferred

- **Tab-switch** and **per-keystroke typing** metrics — dropped from this pass
  because both are dominated by harness/IPC noise (a flaky metric is worse than
  none). Revisit with an in-page driver if they earn their keep.
- A live in-app perf overlay.
