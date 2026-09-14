# Diagnostics content boundary

This contract enforces I-5 (graph-authored content does not enter automatic
logs, crash reports, or telemetry) and I-11 (a diagnostic helper's name and
gate agree). Diagnostics are local only; Tine does not add a telemetry channel.

## Channels

The always-on native flight recorder accepts a fixed event name plus reviewed
boolean, numeric, enum, and public build fields. It accepts no free-form
message, path, detail, page title, block text, property value, or graph-relative
path. `fixed_event_shape_contains_no_free_form_message_fields` in
`src-tauri/src/debug.rs` is the exemplar. Diagnostic reports copy this recorder,
not the directed debug log.

The src-tauri detailed helper `diag` is a no-op unless `debug_enabled()` is true
through `TINE_DEBUG=1` or `--debug`. This is an explicit, local investigation
channel and may contain a directed path or OS error; it is never folded into the
automatic recorder. A src-tauri failure that must remain always-on instead uses
a fixed-shape event or a fixed content-free terminal line.

## One flag answers "are debug diagnostics on"

`TINE_DEBUG` and `--debug` are parsed in exactly one function,
`src-tauri/src/debug.rs::debug_opt_in_requested`. `debug_init` hands its answer
to `tine_core::sync_runtime::set_runtime_debug_diagnostics(bool)` once, at the
top of `run()` and before any subsystem starts, and every later reader — core
and src-tauri alike — asks
`tine_core::sync_runtime::runtime_debug_diagnostics_enabled()`. `debug_enabled()`
in src-tauri is a thin delegate to that same reader, so the two crates cannot
disagree about whether debugging is on (I-12). The setter is plain and
idempotent, not init-once: a test may flip it, and the last call wins.
`exactly_one_function_reads_the_debug_diagnostics_flag` in
`crates/tine-core/tests/content_out_of_logs.rs` fails if a second function
parses the environment or reads the flag directly.

The core crate cannot call the src-tauri event helper. Newly retained core
diagnostics are therefore content-free and use
`runtime_debug_diagnostics_enabled()`. Existing specialized performance traces
remain behind their named environment flags and are census-reviewed as directed
local investigation channels. An always-on core line is legal only when its
allowlist row explains why its error or fixed shape cannot carry user content.
When a core failure must stay visible always-on but its error is prose about a
graph object, split it:
`crates/tine-core/src/direct_projection.rs::report_projection_failure` is the
exemplar — the fixed failure family always-on, the raw error behind the flag.

Frontend `console` output is not captured, persisted, or transmitted, and is
included in no report — but the WebView inspector ships in release builds, so a
console line is one panel away on a user's machine. Variable-bearing calls
require an exact reviewed entry, and a caught failure reaches one only through
`src/failureShape.ts::failureShape`, which keeps a failure's type, message size
and message hash and drops the message itself. Graph identity on the save path
is represented by a count, never a page name.

Every failed page save has one fixed-shape native receipt. Direct Files records
its closed save-failure code and guarded-index counters as `direct.save`;
Tine-managed storage records the actor's closed refusal code as `managed.save`.
Managed conflict and deferred outcomes preserve their closed, content-free
subtype (for example `managed.conflict.stale_base` or
`managed.deferred.blocked_recovery`), rather than disappearing behind a generic
IPC failure.
Unknown actor stages collapse to `actor_refused_at_stage`: stage text, page
identity, paths, error prose, and draft content never enter either event.

## Print-site classes

Every row of both censuses carries one of four buckets, and they mean the same
thing on both sides of the boundary:

| Bucket | Meaning | Where it is legal |
| --- | --- | --- |
| a | Content-free or fixed-shape payload, behind a debug opt-in | anywhere |
| b | A directed investigation channel behind its OWN named opt-in; may carry detail | anywhere |
| c | Always-on, with a variable that CAN carry user content | **nowhere — this class is empty and both ratchets fail if a row claims it** |
| d | Always-on, payload provably content-free | anywhere |

The retained class (b) channels are named, and each is off unless its own flag
is set: `TINE_PHASE_TRACE`, `TINE_CRDT_TRACE`, `TINE_ACTIVATION_TRACE`,
`TINE_BATCH_TRACE`, `TINE_PUBLISH_TRACE`, `TINE_SEMANTIC_TRACE`,
`TINE_TICK_TRACE`, `TINE_CLEAN_WATCHER_TRACE`, and the
`TINE_DEBUG`/`--debug` opt-in behind `runtime_debug_diagnostics_enabled()` /
`debug_enabled()`. `crates/tine-core/src/oplog/projection.rs` holds the one class
(b) line that renders graph bytes; it is legal only because that trace is off by
default and explicitly requested.

This paragraph is pinned in both directions by
`directed_trace_flags_are_the_same_set_in_the_contract_and_in_the_census` in
`crates/tine-core/tests/content_out_of_logs.rs` (I-11): the `TINE_*_TRACE` names
here must be exactly the set the class (b) `gate:` fields of that file's
allowlist carry, and every `TINE_*_TRACE` environment read in compiled
`crates/tine-core/src` production source must be one of them. A directed trace
flag is a contract row, not a free-text gate. The scan is core-only; src-tauri's
directed channel is `debug_enabled()` behind `TINE_DEBUG`, which is pinned
separately by `exactly_one_function_reads_the_debug_diagnostics_flag`.

Class (d) rows are always-on, so each one names why its payload cannot carry
content. The recurring proof is that a `std::io::Error`'s `Display` never
includes the path it failed on; the others carry counts, worker numbers, bounded
enum causes, or — at
`src-tauri/src/data_home.rs` — a `std::io::ErrorKind` token. That last line is a
fatal refusal the user cannot get past, so I-9 requires the failure family to be
identifiable there without a relaunch under `TINE_DEBUG`; `ErrorKind` is a
bounded enum, so it says `PermissionDenied` without the OS prose or the
directory.

## Managed runtime tick vocabulary

The native bridge emits only these bounded tick states: `idle`,
`checkpoint_capture_skipped`, `local_mutation`, `provider_mutation`,
`recovery_blocked`, `recovering`, `retry_full`, `blocked`, `failed`,
`admitted_noop`, `admitted_complete`, and `terminal`.
`checkpoint_capture_skipped` is deliberately distinct from `recovering`:
the disposable checkpoint was not captured, while accepted authority and the
foreground runtime continue unchanged. This vocabulary is pinned by
`checkpoint_capture_skip_has_its_own_tick_value`.

The fixed `managed.checkpoint_capture_skipped` receipt carries exactly one of
`runtime_not_attached`, `indexed_runtime`, `blocked_runtime`,
`unsettled_runtime`, `durable_frontier_ahead`, or `capture_failed`.
These are bounded causes, never error prose, and are pinned by
`fixed_event_shape_contains_no_free_form_message_fields`.

The fixed `managed.history_recovery` receipt carries the recovery attempt,
closed reason and phase, BatchId and DocumentId, requested and actual CRDT
floors, closed retry class, numeric fences for all three journal domains,
accepted/pending/replayed counts, phase durations, checkpoint bytes when a
checkpoint completed, publication edge, and preservation result. It never
copies the free-form retry cause; the frontend applies its existing bounded
sanitizer when that cause is shown or explicitly copied.

The fixed `managed.checkpoint_publication` receipt carries T, E, the age cutoff,
clock-frozen state and last reset time, the policy revision and byte policy,
each changed document's identity/floors and I/L/R/B
measurements, post-cut remainder, hysteresis shortfall, budget overage and
closed limiting cause, changed/exported/reused and worker export counts,
hot/cold read and byte totals, phase durations, checkpoint bytes, and the
commit-last publication edge. Peak RSS is emitted only when measured. These
records contain identifiers and counts, never page names, paths, or document
bytes.

Parser failures cross the lsdoc-diff worker boundary only as a status plus
`ParserDiagnostic`: a nullable numeric offset, UTF-8 input length, and opaque
input hash. An exception object or free-form parser detail cannot inhabit that
type.

## Equality ratchets

`crates/tine-core/tests/content_out_of_logs.rs` walks production Rust library
sources, excluding standalone CLI output and cfg(test) regions. Its exact
allowlist currently contains 75 Rust production print sites, each with a class,
reason, and gate. A deletion changes the census just as an addition does.

`src/contentOutOfLogs.ratchet.test.ts` walks production TypeScript and TSX and
classifies 21 variable-bearing frontend console sites. It also pins the parser
failure shape and the two allowlist counts in this document. Changes to either
census require an explicit contract review.

Both ratchets additionally assert that no row is class (c). That assertion, not
a reviewer's memory, is what keeps the class empty.

Neither census is anchored to line numbers. The Rust rows name a **function and
an occurrence within it**; the frontend rows name a **content digest of the
call's arguments**. Both anchors move with their call, so an edit above a print
or console site changes nothing, and neither census has a mechanical
re-anchoring step. They go red only when a site is added, removed, moved between
functions, or has its payload changed — each of which is a real change to what a
shipped binary can emit and needs a human classification, which is the review
this section requires.

The required repair for an unreviewed Rust site is: use a fixed-shape event
(src-tauri) or a content-free flag-gated line (core).
