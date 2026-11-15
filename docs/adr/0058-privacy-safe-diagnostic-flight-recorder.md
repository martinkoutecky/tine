# 0058. Diagnostics are a privacy-safe flight recorder, persisted in app data

- **Status:** Accepted (og port of master ADR 0058; persistence approved by
  Martin, og QUESTIONS Q5, 2026-09-29)
- **Date:** 2026-09-29

## Context

Many failures happen only on a reporter's Windows or mobile device. A separate
"debug build" does not help: the failure has already happened, and a general
trace can expose paths, page names or note content. og batch 18a ported master's
recorder in memory only, so a report could never describe the run that crashed,
nor say whether the previous run ended cleanly. Master persists the recorder as
rotating JSONL segments appended in place; og writes every durable file through
its audited atomic helper instead.

## Decision

Every build keeps an always-on flight recorder
(`src-tauri/src/flight.rs`). Its API accepts only fixed event kinds, catalogued
command names, closed-vocabulary tokens, booleans, counts and durations. It
never accepts note content, paths, page titles, queries, URLs, credentials,
error text or the opt-in `TINE_DEBUG` log.

**Page route refusals (GH #863).** `page.open-refused` records `stage`
(`route` or `probe`), `readSource` (`disk`, `held-bytes`, `held-absent`,
`held-unindexed`, or `not-read` for a rejected target), and boolean
`pathMismatch`, `nameMismatch`, `kindMismatch`. An absent answer mismatches
all three operands. Provenance accompanies the original answer; it is never
rechecked against later owner state. Successful opens omit this refusal event.
These events use the existing bounded buffer, persistence worker and report
export. They add no path, title, content or timestamp field.

**Editor focus misses (GH #939).** `editor.focus-missed` (frontend kind
`editor_focus_missed`) is written when, one animation frame after
`startEditing` asked a block to edit, no mounted editor of that block holds
focus in a focused document, or a guard refused the request. Fields: `entry`
(`enter-split` or `other`), boolean `documentHasFocus`, `guard`
(`exit-guard`, `entry-guard` or `none`), boolean `targetMounted`,
`activeTag` (`textarea`, `input`, `button`, `select`, `iframe`, `a`, `div`,
`span`, `body`, `none` or `other`; a CodeMirror block editor is a `div`) and
boolean `activeIsBlockEditor`. No block
id, text, title or path. The frontend writes at most one per five minutes per
session and schedules no check inside that window; a newer edit request
supersedes a pending check.

**Workspace-window stages (GH #809).** `workspace.open` carries a process-local
`sequence` (the numeric popup counter, never its label), a fixed `stage`, and
`openElapsedMs`. The usual `elapsedMs` is still time since recorder startup.
Native open time starts at prepare entry; frontend open time starts at launch
and includes its prepare IPC round trip. Compare durations within a side;
async delivery can interleave the two sides' events.

- Frontend: `frontend.prepare-done`, `frontend.open-called`,
  `frontend.open-returned`, `frontend.document-ready`,
  `frontend.bootstrap-done`, `frontend.focus-called`, `frontend.focus-done`,
  `frontend.failed`. Focus events occur only for an activated window. Dispatch
  before open/focus does not await diagnostic IPC; it cannot guarantee delivery
  if the renderer itself stops before dispatch reaches native code.
- Native: `native.prepare-received`, `native.prepared`,
  `native.request-received`, `native.builder-entry`, `native.build-returned`,
  `native.build-failed`, `native.created-geometry`, `native.response-create`,
  `native.page-load-started`, `native.page-load-finished`. Page-load events are
  engine notifications; an engine may not notify an initial `about:blank` load.
  Wry's Windows `SetNewWindow` and deferral completion occur after Tine returns
  `Create` and are **not observable here**. `response-create` only describes
  Tine's handoff, not their completion. No URL or title is recorded.
- `native.prepared` adds rounded `logicalWidth`/`logicalHeight` (1–16384),
  `monitorCount` (0–64) and `targetScaleMilli` (100–16000). The target scale
  before creation is null when OS placement chooses the monitor, or no monitor
  contains the requested top-left corner. `native.created-geometry` adds the
  actual window scale and `physicalWidth`/`physicalHeight` (1–262144).
  Unknown or out-of-range facts are null. No screen position or monitor name
  enters the report. This adds events only; popup creation policy is unchanged.

**Window creation failures (GH #713).** When a desktop window's webview
cannot be built, `window.create_failed` records `window` (`graph`: a graph
window, whose WebView2 environment is created for it; `workspace`: a popup
sharing its opener's environment), `hresult` (`0x` and eight hex digits, parsed
from the error; null when the error carries none), `openWindows` (webview
windows still open) and `buildMs` (duration of the failed build). The error
text itself never enters the report. One event per failed open; none otherwise.

**Native main-loop liveness.** On Linux, Windows and macOS one sleeping worker
posts an acknowledgement every 5 s, with at most one outstanding callback.
That callback only sends an `Instant` through a one-slot channel and never
touches the flight recorder. The worker records `runtime.main_thread_stall`
with `thresholdMs` (2000, 10000 or 30000), measured `delayMs`, and `recovered`.
Each threshold is recorded once while waiting; after 30 s it sleeps until the
reply, with no repeated events. A late reply records recovery. Idle probes
produce no recorder events, no dirty flag, no disk writes: one thread, one
channel/queued callback per 5 s, no polling loop. Android and iOS explicitly
compile out the worker because OS suspension is expected; a guard test pins
all five target names. A native stall does not establish a root cause, and
missing frontend progress with a quiet native watchdog suggests a renderer
stall only if the worker had time to probe.

For a freeze, wait **60 seconds** before force-closing, relaunch once and send
the diagnostics report: `sessions.previous` retains flushed stages and native
stalls, and `previousExitUnclean` distinguishes the forced end. Persistence
continues on its existing background worker even when the native loop hangs.
The Linux kill/relaunch journey uses a `cfg(test)` hook at popup builder entry;
shipped binaries contain no blocking hook or environment switch for it.

**Persisted format** (`src-tauri/src/flight_store.rs`). One directory,
`<app data>/diagnostics/`, never inside a graph:

| File | Contents | Written |
|---|---|---|
| `history.jsonl` | this run's retained events, one JSON object per line (`schemaVersion` 1, `elapsedMs`, `event`, fixed fields), at most 1 MiB (1,048,576 bytes) including newlines | at launch; every 30 s while new events arrived; at a panic; at an orderly exit; when a mobile app is hidden or shown; after Clear |
| `session-active` | empty; present while a session may die unexpectedly | created at launch and when a hidden mobile app returns; removed at `RunEvent::Exit` and when a mobile app is hidden |
| `process.lock` | empty; advisory lock of the one process that owns the directory | created once |

Every write is `device_io::atomic_write` (temp + fsync + rename + directory
sync). At launch the previous run's `history.jsonl` is read once (at most
1 MiB), kept in memory as the report's `previous` session, and replaced by this
run's history. A report therefore covers this run and the previous one.

**Unclean exit.** A `session-active` marker present at launch means the previous
run ended without an orderly end: the report says `previousExitUnclean`, and a
sticky toast offers Settings → Help & diagnostics. Desktop's orderly end is
`RunEvent::Exit` (`App::run` never returns, master d9763603). On Android and iOS
the OS reaps a hidden app without notice, so the session follows visibility
(master a846665f, GH #426); desktop ignores visibility, because a crash behind a
minimised window is exactly what the recorder must catch. On Linux and
macOS the first SIGTERM, SIGHUP or SIGINT (shutdown, logout, a closed terminal,
Ctrl+C) runs the tray's Quit and, after 5 s, destroys the graph windows whose
webview could not answer (the session's signal kills WebKit's helpers too);
either way the exit goes through the retirement waiter to `RunEvent::Exit`.
Later signals are ignored (GH #701, `src-tauri/src/quit_signal.rs`).

**Saved report.** Settings → Help & diagnostics can Copy the report, or on
desktop Save it to a file the user picks in the save dialog, written with the
same atomic helper. Nothing is uploaded.

**Refusals.** Startup is never refused. A `process.lock` held by another live
process leaves this process recording in memory only (scenario: honest
concurrent Tine instance — a forwarded second launch must not rotate the
primary's evidence). A history file that is unreadable, larger than 1 MiB or
holds malformed lines keeps only its whole valid events and is rebuilt by the
launch's first write (scenario: crash/power loss or disk error leaving a
partial file; the history is a disposable cache).

## Unit cost

Unit cost: 0 graph bytes and 0 files per edit, on a 1-block and a 60-block page
alike: a successful save under 150 ms records no event, so the recorder does not
become dirty and nothing is rewritten (measured: 1,000 such saves leave the
recorder clean, `ordinary_saves_cost_no_diagnostic_bytes`). Per launch: one
`history.jsonl` rewrite of the events so far (measured 254 bytes for a probe
launch with one watcher event and a clean exit), one empty marker, and reading the previous history once (≤ 1 MiB).
Per flush: one file rewritten, O(retained bytes) ≤ 1 MiB, at most once per
30 s and only after a new event. Per exit: one history rewrite plus one marker
removal. Steady state: 2 files (`history.jsonl` ≤ 1 MiB, `process.lock` 0 bytes)
plus the marker while running. Transport bytes: 0 (nothing is sent). Measured by
the child-process probes in `flight.rs`.

## Consequences

A report now describes the run that crashed and whether it ended cleanly,
without a special build. A kill (not a panic) loses at most the last 30 s of
events; a panic publishes at once. The field vocabulary and the separation from
the detailed log remain security boundaries and stay contract-tested. Not
ported from master: rotated `*-old` segments (og keeps one 1 MiB history per
run), storage-transition and index-failure events (og has no index/readiness
machinery), and release-CI symbol retention.
