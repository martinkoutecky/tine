# Native og parity bench

Run after a fresh frontend build and release binary build:

```sh
rtk npm run build
rtk cargo build --release -p tine --features custom-protocol
rtk proxy xvfb-run -a node scripts/bench-og-parity.mjs
```

The runner copies og's fresh binary and the installed master reference into
`test-results/og-bench/binaries/` before launching either one. It verifies
the master's revision and SHA-256 against `~/research/tine.build.json`.
It generates 2,000 and 10,000 synthetic pages with seed 543 from master's
`generate-realistic-graph.mjs` (copied into `scripts/`), plus a private copy of
`~/research/logseq-anonymized`. Each corpus receives a synthetic hub and 200
referrer pages; each measured trial gets a private graph copy, with an
independent copy for rename so a save conflict cannot affect it. The original
anonymized graph is never opened for writing or quoted in output.

Five interleaved runs per binary and corpus produce `summary.json`,
`comparison.md`, and one `result.json` per trial. Timings use milliseconds:
launch to first rendered content; quick-switch block search; click to page
paint; linked and unlinked reference counts; ten editor keypresses to the next
paint (median and p95); edit event to disk; and rename of the synthetic hub
with 200 referrers. The 10k trials also report app RSS after the first
graph-wide search has completed the background load, and after the save and
rename journeys, with their larger post-journey RSS used for the
overall after value. JSON holds all samples; the table shows median and min–max
spread. A failed journey has no made-up latency and is named under
`journeyFailures`.
Failure screenshots are stored beside the trial result. To rebuild the table
from completed trial records without rerunning the app, set
`TINE_OG_BENCH_SUMMARIZE_ONLY=1` with the same output directory and run count.
`scripts/export-og-bench-baseline.mjs` copies the table into `docs/` and writes
a numerical JSON copy without trial messages or graph content.

WebKitGTK on this Linux runner does not expose `PerformanceObserver`'s
`longtask` entry type. The runner uses gaps between animation frames; every
gap over 100 ms is listed by journey and the maximum is reported. The probe
starts as soon as WebDriver returns the session, so very early startup tasks
before that point are outside its observation window. The 500 ms UI budget
applies to every corpus and journey. A trial over it fails the bench. The
typing p95 budget is under 16 ms. `taskset` can pin the whole bench when load is
high, for example `rtk proxy taskset -c 2-5 xvfb-run -a node scripts/bench-og-parity.mjs`.

Optional environment variables: `TINE_OG_BENCH_OG`, `TINE_OG_BENCH_MASTER`,
`TINE_OG_BENCH_ANON`, `TINE_OG_BENCH_OUT`, and `TINE_OG_BENCH_RUNS` (at least
five). `TINE_OG_BENCH_CORPORA` selects `2k,10k,anonymized` for a focused
diagnostic run. `TINE_OG_BENCH_PILOT=1` permits a one-run harness pilot; its
numbers are not baseline results.
