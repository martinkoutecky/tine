# PDF viewer perf harness and release gate

`scripts/perf-pdf/` drives the real Tine release binary through tauri-driver
and measures what a reader of the PDF viewer sees: how much of the viewport is
blank, how long frames take, how quickly the view becomes sharp, how far a zoom
moves the point under the pointer, and how much memory the web process takes.
It runs before every stable cut as a gate that compares the candidate binary
with a base release on the same machine (decision D-20261021-PV2). The base is
the newest published release, stable or Beta, that carries the PDF probe
(D-20261021-PV4): releases before the probe give exit 4, no verdict.

## What it measures

The viewer reports its raster lifecycle through a no-op hook,
`src/components/pdfProbe.ts`: render start/end/abort, page free, tile
start/end/remove, text layer, zoom input and zoom settle. The hook costs nothing
unless the harness installs `globalThis.__tinePdfProbe`. A sampler
(`collector.js`) runs on every animation frame and splits the visible viewport
into three classes:

- **blank**: no completed raster (no canvas yet, or a re-raster in flight that
  cleared it), except where finished tiles at the current scale cover it;
- **stale**: a completed raster at an older scale, shown CSS-scaled;
- **sharp**: rasterized at the current scale at the viewer's own target
  resolution.

Metrics per trial (`metrics.mjs`): blank ms (blank fraction × frame duration,
summed), p95 and max inter-frame interval, frames over 100 ms, time to sharp,
anchor drift for zoom (the distance the document point under the pointer
moved), and the peak RSS of the app's WebKit web processes from `/proc`.

The same scenarios can run in the stock pdf.js 4.10.38 viewer loaded into the
same webview (`--viewers ours,stock`); that comparison is the reference for
future viewer work. The gate measures Tine's viewer only.

## Inputs

`node scripts/perf-pdf/assets.mjs` downloads the corpus and the pdf.js release
once into `~/.cache/tine-pdf-perf` (`$TINE_PDF_PERF_CACHE` or `$XDG_CACHE_HOME`
override it) and verifies each file against a pinned SHA-256. The harness and
the gate call it themselves. `PDF_CORPUS` and `PDFJS_DIST` point at existing
copies instead. Nothing of this is committed.

| id | document | source |
|---|---|---|
| S1 | two-column IEEE paper, 23 pp | arXiv 0807.3917v5 |
| S2 | textbook, 714 pp, 6.9 MB, outline | Boyd & Vandenberghe, *Convex Optimization* |
| S3 | scanned book, 748 pp, 34 MB, JPX scans | archive.org `sourcebookinmath1929smit` |

## Host requirements

Linux with Xvfb (`xvfb-run`), `tauri-driver`, `WebKitWebDriver`
(`/usr/bin/WebKitWebDriver` or `$WEBKIT_DRIVER`), `xdotool`, node and the
repository's `node_modules`. Under Xvfb the harness runs WebKit with software
GL and compositing disabled, so absolute frame times are worse than on a
desktop; base and candidate run under the same conditions. `PERF_HOST_GL=1`
keeps the host's GPU path for a run on a real X11 desktop.

Only one harness session may run on a machine at a time: a second one shares
the CPUs and the driver ports. The gate refuses to start while another
`perf-pdf/run.mjs` runs. Run no builds while it measures.

## The gate

```bash
node scripts/perf-pdf/gate.mjs --base <newest probed release binary> --candidate target/release/tine
```

Both arguments are Linux release binaries built with `custom-protocol` (the
deployed raw binary, or the `tine` binary of a release build). Options:
`--out <dir>` (default `target/perf-pdf-gate/<timestamp>`), `--sessions 3`,
`--dprs 1,2`, `--load-max 4`, `--load-wait-min 15`, `--cpus <taskset list>`.
`--report-only <out>` re-renders an earlier run's report from its raw rows.
Only one gate runs at a time (a lock under the temp directory).

It runs a trimmed scenario set (`run.mjs --gate`) in fresh sessions, one Tine
launch on a fresh graph each:

- open S1, S2 and S3 (time to sharp, blank, frames);
- S1a: S1 at fit width, scrolled top to bottom in-page at 7000 and 2000 px/s;
- S1c: five-step zoom bursts at mid-page, Ctrl+wheel and the toolbar buttons,
  in and out, three cycles each;
- S2: 50 pages of S2 at 7000 px/s, in-page and with xdotool wheel input.

Each runs at DPR 1 and 2, three sessions per binary. Base and candidate
sessions alternate (base first in odd repetitions, candidate first in even
ones) so machine drift hits both. Before every session the gate waits until the
1-minute load average is below `--load-max`; if it stays above for
`--load-wait-min` minutes the gate aborts. Every session's load is in the
report. Expect about 20 minutes on an idle 12-thread machine.

### Reading the verdict

`<out>/report.md` and `report.json` hold the outcome, every metric, and the
sessions. Raw rows, one dump per trial, and one log per session are in
`<out>/raw/`.

Every metric is reduced to one value per session (S1c: the median of its three
cycles), so each side has three values per cell and DPR. A metric
**regresses** when the candidate's best session is worse than the base's worst
session by more than

    max(base range, floor, rel × base median)

That is, the run-to-run ranges do not overlap and the gap between them is
larger than the base's own spread in this run, than an absolute floor and than
a fraction of the base value. Improvements mirror this. A median shift beyond
the threshold whose ranges overlap is listed under **watch** and does not fail
the gate. A candidate that loses a value the base has (for example, never
became sharp) counts as a regression.

| family | floor | rel |
|---|---|---|
| blank ms | 150 ms | 25 % |
| p95 frame | 15 ms | 20 % |
| max frame | 100 ms | 25 % |
| frames > 100 ms | 3 | 50 % |
| time to sharp | 80 ms | 25 % |
| anchor drift | 25 px | 20 % |
| web RSS peak (per session) | 64 MiB | 10 % |

The floors come from same-binary (A/A) comparisons on the reference machine
(Ryzen 5 8600G, 6 cores × 2 threads, WebKitGTK 2.50.6, Xvfb): 14 pairs of
same-binary session triplets, taken from three gate runs 20–50 minutes apart.
None flags with these floors; the largest same-binary gap reaches two thirds of
its threshold. Two families needed floors above the first guess: blank time on
a fast S1 scroll at DPR 1 varies 46–472 ms between runs of one binary, and a
zoom burst's max frame is bimodal (about 77 ms, or one 140–180 ms frame), so a
session median of three cycles can land in either mode. With these floors the
old viewer, measured as a candidate against the current one, is still flagged
for blank-on-zoom at DPR 2, zoom-anchor drift, zoom frame times and scroll
blank time (33 metrics); its 45 ms zoom blank at DPR 1 is below the blank
floor and is caught there only through frame times and drift.

Exit codes:

| code | meaning |
|---|---|
| 0 | pass: no regression |
| 1 | regression: stop and consult, never loosen a threshold to pass |
| 2 | incomplete: a session failed or a trial errored, or the candidate has no probe |
| 3 | aborted: the machine stayed loaded |
| 4 | no verdict: the base has no probe (a release from before the probe shipped); the report covers the candidate only |

A base binary without the probe is detected on its first open (no probe event
within 15 s); its remaining sessions are skipped.

## Running scenarios by hand

```bash
LANG=C.UTF-8 LC_ALL=C.UTF-8 xvfb-run -a -s "-screen 0 1920x1080x24" \
  node scripts/perf-pdf/run.mjs --dpr 1 --scenarios S0,S1a,S1b,S1c,S2,S3 --out <dir> [--viewers ours,stock] [--theme dark]
node scripts/perf-pdf/report.mjs <dir> <tag-prefix> > tables.md
```

For DPR 2 use a `3840x2160x24` screen and `--dpr 2`. `TINE_APP` selects the
binary (default `target/release/tine`) and `PERF_PORT_BASE` the driver ports.
S0 is a sanity check (idle reads no blank; a jump to an unrendered page reads
the whole viewport blank); S1b is zoomed column reading at 2.57 and 3.42 CSS
px/pt; S2 also has page-field and outline jumps; S3 scrolls the scanned book.

Known limits: fast wheel scrolling on S1 is not a throughput measure (WebKit
holds the wheel animation in some states under Xvfb), so the gate takes wheel
throughput from S2 only. WebKitGTK delivers no Long Tasks entries, so frame
times come from animation frames. Stock pdf.js's progressive first raster
counts as blank.
