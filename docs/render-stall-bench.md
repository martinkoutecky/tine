The renderer typing bench (GH #746) generates 40 linked and 156 unlinked
references with long accented and CJK text. It expands both sections, then
sends a native WebDriver held-key sequence at roughly 30 keys/s for three
15-second runs. It records frame gaps independently of IPC latency.

Build and run on Linux:

```sh
source scripts/env.sh
npm run build
cargo build --release --features custom-protocol -p tine
TINE_APP=target/release/tine STALL_OUT=artifacts/render-stall/local \
  xvfb-run -a -s "-screen 0 1920x1080x24" node scripts/e2e-render-stall.mjs
```

Use a fresh output directory for each binary. `report.json` reports blocking
milliseconds per second (sum of frame gaps over 50 ms, minus one 60 Hz frame
per gap, divided by measured elapsed seconds), maximum input gap, frames over
100 ms and maximum frame gap. Raw samples and a screenshot accompany it.
This is a frame-gap approximation of main-thread blocking, not CPU profiling.
The typing phase excludes initial graph loading and reference expansion.

`STALL_RUNS`, `STALL_SECONDS`, `STALL_PAD` and `STALL_SETTLE_MS` configure
calibration. Default padding produces 11.5 MB of UTF-8 mention text, before DTO overhead.
The default budget is provisionally 100 blocking ms/s, calibrated on Linux:
fixed `88c82baf7` measured at most 29.2 ms/s in three heavy runs, while
`56bd13337` measured 573 ms/s and failed. `STALL_BUDGET_MS_PER_SECOND` overrides
the budget; an explicitly empty value selects measurement only. Do not treat
measurement-only calibration as a performance gate. Windows calibration is
still required before certifying this as a shared cross-platform budget.
See the implementation receipt for all measurements and limitations.

The **Renderer typing stall bench** GitHub workflow runs WebView2 on hosted
Windows only when manually dispatched. Its optional budget input has the same
meaning. It has no push trigger. Running or dispatching it is left to the
maintainer.

`STALL_VERIFY_EXPORT=1` also checks the native Settings report: after timing
ends it injects a known 150 ms frame stall, opens Help & diagnostics and checks
that the existing report includes the bounded snapshot. The injected stall is
outside the reported benchmark numbers. `STALL_PAD=1 STALL_SECONDS=2 STALL_RUNS=1`
is enough for this plumbing check, but cannot certify the performance budget.

Normal app diagnostics now include an export-only `renderer` snapshot: up to
32 worst stalls of at least 100 ms during typing. Chromium adds Long Animation
Frame script durations, bundled asset filename/character position, and Event
Timing where available, observed only from the first key until 2 s without
input. Every engine records key-to-next-frame latency: a burst of keys requests
one animation frame. There is no per-frame loop, because a continuous
requestAnimationFrame chain after each key cost Chromium about a frame on the
next large-page load and scroll (bench, 8554ec5a1). Background frames are
ignored. No key value, note text, DOM target, full script URL or function name is
recorded. The snapshot covers the app window and its workspace windows, stays
in memory, and is cleared with
**Clear recorded events**; restarting discards it. Copy and Save use the existing
diagnostic export path.

## Load sensitivity

The budget only means something on an otherwise idle machine. On the Linux
development host, the same fixed build measured 11–35 ms/s at load average
below 1.5, but 90–133 ms/s at load 3.4 (a parallel build running). Run the
bench with nothing else compiling or testing, and A/B two builds alternately
in one session rather than comparing against numbers from another time. It is
therefore not part of the integration gate; it is a release-candidate and
investigation tool.
