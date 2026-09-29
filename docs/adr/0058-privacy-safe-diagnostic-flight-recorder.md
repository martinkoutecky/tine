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
minimised window is exactly what the recorder must catch.

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
