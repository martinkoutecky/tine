# Storage A/B performance harness (step-3 gate 2)

`scripts/bench-storage-ab.mjs` is the performance A/B of `STEP3-DESIGN.md` section 13: a
baseline and a candidate Tine binary, run interleaved on private copies of the same graph, with
medians, p95 and a measured A/A noise floor per metric. It replaces the `ddf408c55` master
reference shape of `scripts/bench-og-parity.mjs` (that script stays as is for the og-vs-master
parity run). It is a sibling script, not an edit of that one.

## One command

```bash
export LANG=C.UTF-8
nice -n 5 xvfb-run -a -s "-screen 0 1920x1080x24" node scripts/bench-storage-ab.mjs \
  --baseline <base>/tine   --baseline-receipt  <base>/tine.build.json \
  --candidate <cand>/tine  --candidate-receipt <cand>/tine.build.json \
  --runs 7 --noise-floor <aa-out>/summary.json --out <ab-out>
```

- Output: `<out>/comparison.md` (table, pooled samples, unmeasured metrics, failures),
  `<out>/summary.json`, one `trials/<scenario>-<arm>-NN/result.json` per trial.
- The corpus is always a COPY of `~/research/logseq-anonymized` plus bench fixtures (`--anon` or
  `TINE_AB_ANON` to point elsewhere; `--corpus 2k|10k` for generated graphs). Every trial gets its
  own copy, its own `HOME` and `XDG_*` dirs, its own driver processes. The original is never written.
- The binaries are copied into `<out>/binaries/<arm>/tine`, and each receipt (`tine.build.json`
  from `scripts/deploy.sh`) is checked against the copy's SHA-256; a mismatch reports the revision
  as unverified.
- One bench process at a time; run it at `nice -n 5` and with `--max-load 6` (each trial waits up to 10 minutes for the 1-minute load average to fall below it; other lanes build on this machine and one A/A whose trials started at load up to 47 gave a 39-51 % spread on the rename metrics). The report prints the load
  average at start and end.
- `--runs` is at least 5 (the stability budget); `--pilot` allows fewer and is for harness
  debugging only. Arms alternate who goes first each run.
- The A/A validation is the same command with `--aa <binary>` (same binary on both arms; the
  noise floor is `|median(A)-median(B)| / mean(medians)`). Metrics above 10 % are marked NOISY and
  an A/B verdict on them is suppressed.
- `--scenarios default|all|a,b,...`; `--list` prints scenarios and metrics. `draftscale` and
  `session` are outside the default set (long). `--session-minutes` sets the session length
  (spec: 30) and `--session-runs` how many sessions per arm (default 1; a slope from one pair has no spread).
- `--summarize-only --out <dir>` recomputes the report from stored trials.

## Save completion is never file readability

Each arm is observed through one adapter yielding the same neutral records (publishes, save spans,
durable drafts, IPC round trips).

- **`ipc` (the base, and the og-storage head `e19281dd7`, which still run the old engine).** The
  base writes no log line at publication: `flight.rs record_save` records only failed or >= 150 ms
  saves, without a timestamp, and `commands.rs log_save_kinds` runs BEFORE the save. The publish
  signal is therefore the `save_pages` IPC response: the command returns only after
  `tine_graph_features::pages::save_pages` has written, synced and indexed the page, and the wire
  value is `{"ok":[...]}` on success. The harness wraps `window.fetch` and matches the typed
  token (`qzx...`) against request bodies by longest common prefix, so a save that captured an
  earlier prefix publishes only the keys it carried. Drafts are `store_draft` responses.
- **`events` (the candidate).** A bench-only NDJSON file, below.

## Candidate bench events (to implement in lane 3b)

Enabled by the environment variable `TINE_BENCH_EVENTS=<absolute file path>`, read once at process
start. Unset: no code path changes and nothing is written. The app appends one JSON object per
line (single `write` on an `O_APPEND` descriptor so lines are never torn; no fsync; best effort;
the writer must never add latency to the save path, e.g. a channel to a writer thread). Every
line has `ev` and `t` (epoch milliseconds, float, `SystemTime`). Text fields carry at most the
first 65536 bytes of the page text (a lossless UTF-8 prefix); `bytes_len` carries the full size.
The harness types into the FIRST block, so the typed marker is always inside that prefix.

| `ev` | extra fields | emitted when |
|---|---|---|
| `save_begin` | `key`, `path` | the Host starts persisting a page version (`key`: opaque, unique per save, equal on the matching `published`) |
| `published` | `key`, `path`, `version`, `bytes_len`, `text` | the Published event of section 13: the version is on disk (rename and the required syncs done) and recorded as Published in the Host |
| `draft_durable` | `path`, `text` | a draft record for the page is durable (fsync, rename and directory sync of the draft store done) |
| `custody_complete` | `path` | a deletion's custody is complete: source removed and the trash entry durable |
| `mail_parse` | `path`, `parse_us` | one page-mail DTO parse finished, wall time in microseconds |
| `host_stats` | `events_len`, `events_bytes` | every 5 s and at close: length and approximate bytes of `Host::events` |
| `launch_recovered` | `drafts` | launch recovery finished; the number of drafts recovered |
| `unfreeze` | (none) | the UI unfroze after a carry-over sequence |
| `index_published` | `path`, `version` | optional: the index adapter consumed a publication |

Example lines:

```
{"ev":"save_begin","t":1791581328668.4,"key":"b1:Bench Save:17","path":"pages/Bench Save.md"}
{"ev":"published","t":1791581328690.9,"key":"b1:Bench Save:17","path":"pages/Bench Save.md","version":17,"bytes_len":212,"text":"- qzxa1b2c3...\n"}
```

Everything that reads these events is **candidate-only and UNVALIDATED** until a candidate
binary exists: the admission round trip (`page_submit` is timed from the page, so it needs no
event but does need the candidate's command surface), carry-over unfreeze, mail parse cost, the
event-vector size and the recovered-draft count. The pilot on the base alone exercises none of
them.

## What is measured

`--list` prints every metric with its unit and meaning. By scenario:

| Scenario | Measures | Notes |
|---|---|---|
| `launch` | cold launch to first page and to first editable block; RSS | median of runs |
| `typing` | keystroke to Published on a 1-block, 60-block and 1500-block page, isolated (tail after the last key) and during a 5 s burst (10 keys/s); lost keys; typing latency (input to next paint) p50/p95 overall and during a running save; long tasks | the 1500-block page is where a save is long enough to type during |
| `delete` | source gone, trash entry present, "Deleted" toast | base trash: `<graph>/logseq/.tine-trash/pages` |
| `rename` | referrers rewritten (200), file moved, new page shows its linked references | |
| `external` | external edit burst over a held and 20 unheld pages: time until visible; no conflict banner on clean pages; clean held page not rewritten | |
| `blockref` | picker Enter to the reference shown; target's `id::` published | |
| `custody` | last key to draft durable with saves failing (`pages/` read-only) and under an external replacement | |
| `drafts` | launch with 0 and 20 kept drafts: first page/editable, RSS, offer visible (base: sticky toast), drafts listed, first draft's text present | recovery is simulated with a hard kill of the process group and a relaunch on the same dirs |
| `draftscale` | the same over count (1, 10, 50) and bytes (small vs 60-block pages) | long; not in the default set |
| `carry2`, `carry5` | carry-over from 2 and 5 sources: tasks shown and published; candidate: click to unfreeze | |
| `unitcost` | bytes, files created/touched, renames, fsync and directory-sync calls per edit | next section |
| `session` | RSS slope over a scripted session; candidate: event-vector size | `--session-minutes` |

Typing is paced by an absolute schedule so a slow tick does not stretch the burst. Long tasks come
from a requestAnimationFrame gap above 100 ms because WebKitGTK exposes no `longtask` observer.

## Unit cost

`scripts/lib/iotrace.c` is a small ptrace tracer (`strace` is not installed here). It attaches to
the running app, follows threads, and logs `open/openat/creat`, `write*`, `fsync/fdatasync`,
`rename*`, `unlink*`, `mkdir*`, `truncate` events as NDJSON (timestamp, syscall, path, byte count,
return value), keeping only paths under the trial's graph dir and XDG dir. It needs Linux
x86_64 with ptrace allowed and `gcc`; the scenario fails with a clear message otherwise. The
harness cuts the trace into windows per edit and reports, per window: write bytes (split into
graph and app-data), write calls, files created and touched, renames, file syncs, directory syncs,
unlinks, mkdirs. Failed syscalls are counted as `failed`, never as work. Windows:

- `edit1`, `edit60`: one isolated edit of a 1-block and a 60-block page, healthy disk
  (median of 5);
- `risk1`, `risk60`: the same with saves failing, so the cost is the draft path;
- `burst1`: a 5 s continuous burst; `delete`: one deletion with its custody;
- `startup`: what the app writes after launch (the base writes a graph backup under app data for
  about 10 s), and `idle10s`: a 10 s quiet window after startup, the noise floor of every other
  window.

`ipcCalls` per edit comes from the page's command log. Transport bytes per edit do not exist in a
local app; the IPC payload is a request body to a localhost custom protocol, not a sync transport.

## Known limits

- The harness measures wall-clock UI behaviour on a shared machine. The A/A run states the
  per-metric noise floor; a verdict is only made against it.
- A candidate whose events file never appears yields no publishes, and the typing scenarios fail
  with "no publish carrying N typed characters" instead of reporting zero latency.
- `draft_durable` for the base is the `store_draft` response, which is the base's own
  acknowledgement, not a file observation.
