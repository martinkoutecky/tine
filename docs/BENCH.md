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

## Deferred

- **Tab-switch** and **per-keystroke typing** metrics — dropped from this pass
  because both are dominated by harness/IPC noise (a flaky metric is worse than
  none). Revisit with an in-page driver if they earn their keep.
- A live in-app perf overlay.
