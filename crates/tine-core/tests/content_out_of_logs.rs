#[path = "support/production_source.rs"]
mod production_source;

use production_source::{
    compiled_source, line_of, production_source_files, relative_path, repo_root, test_only_include,
};
use regex::Regex;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The census answers "what does a shipped binary compile?", so a file the
/// binary never compiles must never reach it — and the two ways a test-only
/// file hides from a naive scanner both exist in this repository today.
///
/// This guard is here rather than left to the print census because when the
/// scanner regresses, the census reports "a print site was ADDED", which sends
/// the reader to the wrong file entirely. It cost a full debugging cycle once.
/// The rule, if this fails: `#[path]` resolves against the DECLARING file's
/// directory, not the included file's, and test-only-ness is TRANSITIVE —
/// a `#[cfg(test)]`-only module passes it on to everything it declares.
/// The blessed exemplar is `tests/support/production_source.rs`.
#[test]
fn the_source_census_excludes_test_only_modules_declared_from_anywhere() {
    let root = repo_root();
    // Declared from the directory ABOVE it, by `src/query.rs`.
    let from_the_parent_directory = root.join("crates/tine-core/src/query/oracle_gate1_tests.rs");
    // Declared with no `#[cfg(test)]` of its own, by a file that is itself
    // reached only under `cfg(test)`.
    let through_a_test_only_declarer =
        root.join("crates/tine-core/src/rebaselining_layout_tests.rs");
    // A `#[path]` include that IS production, so the rule cannot simply be
    // "anything pulled in by `#[path]` is a test".
    let production_by_path = root.join("crates/tine-core/src/oplog/sealed_document_map.rs");

    for file in [&from_the_parent_directory, &through_a_test_only_declarer] {
        assert!(
            file.is_file(),
            "{} moved; repoint this guard at whatever kept its declaration shape",
            relative_path(&root, file)
        );
        assert!(
            test_only_include(file),
            "{} is compiled only under cfg(test), but the source census counts it as \
             production: `#[path]` resolves against the DECLARING file's directory, and \
             test-only-ness is transitive. See tests/support/production_source.rs.",
            relative_path(&root, file)
        );
    }
    assert!(
        production_by_path.is_file() && !test_only_include(&production_by_path),
        "{} ships in the binary; excluding it would hide real sites from every census",
        relative_path(&root, &production_by_path)
    );
}

/// A print site's IDENTITY: which function it lives in, and which of that
/// function's prints it is. Deliberately NOT the line number — see the note on
/// `ALLOWLIST`.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PrintSite {
    file: String,
    function: String,
    macro_name: String,
    occurrence: usize,
}

impl PrintSite {
    fn identity(&self) -> String {
        format!(
            "{}::{}#{} {}!",
            self.file, self.function, self.occurrence, self.macro_name
        )
    }
}

#[derive(Clone, Copy)]
struct AllowedSite {
    file: &'static str,
    function: &'static str,
    macro_name: &'static str,
    /// Which of this function's prints, in source order, this row classifies.
    occurrences: &'static [usize],
    /// One of four buckets, the same four `src/contentOutOfLogs.ratchet.test.ts`
    /// uses, so one classification answers "is this line safe?" on both sides:
    ///
    ///   a - content-free or fixed-shape payload, and gated behind a debug opt-in.
    ///   b - a directed investigation channel behind its OWN named opt-in; may
    ///       carry detail, because the user asked for it.
    ///   c - always-on, with a variable that CAN carry user content (a page name,
    ///       a graph path, block text, or error prose from an operation over any
    ///       of those). MUST BE ZERO; `class_c_is_zero` enforces it.
    ///   d - always-on, payload provably content-free.
    bucket: &'static str,
    class: &'static str,
    why: &'static str,
    gate: &'static str,
}

// Production app output only. Standalone CLI binaries own their terminal
// output and are outside the application diagnostics contract.
//
// Rows are anchored to a FUNCTION and an occurrence within it, not to a line
// number, and that is the whole point of the `function` column.
//
// This census used to pin `{ file, lines }` — 76 absolute line numbers, half of
// them in `model.rs`, `hot_engine.rs` and `sqlite.rs`, the three files every
// lane edits. Inserting a line anywhere above one of them turned the census red
// for work that never touched logging, and the repair was a script that
// re-anchored the numbers mechanically. A guard whose normal failure mode is
// "run the fixer" is a guard nobody reads: the I-12 test thirty lines below
// already says exactly this in its own comment, and already anchors on the
// function. This now does the same, and `scripts/reanchor-print-census.mjs` is
// deleted rather than kept as a tempting shortcut.
//
// A function name is also a better classification unit than a line: `why` can be
// checked against the function it names, which a line number cannot support.
// The census still moves when a print is ADDED to, REMOVED from, or MOVED
// BETWEEN functions — each of which is a real change to what a shipped binary
// can emit, and each of which deserves a human classification.
const RUST_PRINT_SITE_COUNT: usize = 75;
const ALLOWLIST: &[AllowedSite] = &[
    AllowedSite { file: "crates/tine-core/src/concord_ledger.rs", function: "run", macro_name: "eprintln", occurrences: &[0], bucket: "d", class: "content-free-error", why: "best-effort ledger update failure carries only a std::io::Error, whose Display never includes the path", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/direct_projection.rs", function: "projection_worker", macro_name: "eprintln", occurrences: &[0], bucket: "d", class: "content-free-error", why: "projection directory creation carries only a std::io::Error, whose Display never includes the path", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/direct_projection.rs", function: "projection_worker", macro_name: "eprintln", occurrences: &[1], bucket: "d", class: "content-free-error", why: "projection lease acquisition carries only a std::io::Error, whose Display never includes the path", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/direct_projection.rs", function: "projection_worker", macro_name: "eprintln", occurrences: &[2], bucket: "a", class: "fixed-debug", why: "the deferred-turn line is the ELSE arm of is_reportable_failure, so the only ProjectionRefusal that reaches it is AwaitingFullInventory, whose Display is a fixed literal; Failed(_) takes the reviewed report_projection_failure path instead", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/direct_projection.rs", function: "report_projection_failure", macro_name: "eprintln", occurrences: &[0], bucket: "d", class: "fixed-family-report", why: "report_projection_failure's always-on line is the fixed failure family and no error value", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/direct_projection.rs", function: "report_projection_failure", macro_name: "eprintln", occurrences: &[1], bucket: "b", class: "directed-core-detail", why: "report_projection_failure's detail line repeats the family with the raw error, for a directed investigation", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/model.rs", function: "force_save_page_at_revision", macro_name: "eprintln", occurrences: &[0], bucket: "a", class: "fixed-debug", why: "Guide-page refusal messages are fixed literals", gate: "cfg(debug_assertions)" },
    AllowedSite { file: "crates/tine-core/src/model.rs", function: "isolate_page_parse", macro_name: "eprintln", occurrences: &[0], bucket: "a", class: "content-free-debug", why: "reconcile and isolated-parse failures contain no path, title, content, or raw error", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/model.rs", function: "load_all_pages_with_permit", macro_name: "eprintln", occurrences: &[0], bucket: "d", class: "numeric-shape", why: "isolated search worker panic reports only a worker number", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/model.rs", function: "save_page", macro_name: "eprintln", occurrences: &[0], bucket: "a", class: "fixed-debug", why: "Guide-page refusal messages are fixed literals", gate: "cfg(debug_assertions)" },
    AllowedSite { file: "crates/tine-core/src/model.rs", function: "sync_file", macro_name: "eprintln", occurrences: &[0], bucket: "a", class: "content-free-debug", why: "reconcile and isolated-parse failures contain no path, title, content, or raw error", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/oplog/batch.rs", function: "retained_bytes", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "numeric-trace", why: "batch composition contains public enum kinds and numeric sizes only", gate: "TINE_BATCH_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/checkpoint_generation.rs", function: "publisher_loop", macro_name: "eprintln", occurrences: &[0], bucket: "d", class: "content-free-error", why: "a failed cut retries at the next trigger; the error is a bounded literal or an object-store error over digest-named private blobs", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/oplog/checkpoint_generation.rs", function: "spawn_publisher", macro_name: "eprintln", occurrences: &[0], bucket: "d", class: "content-free-error", why: "publish_capture's error is a bounded literal or an object-store error over digest-named private blobs", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/oplog/hot_engine.rs", function: "drain_staged", macro_name: "eprintln", occurrences: &[0, 1, 2, 3], bucket: "b", class: "directed-core-trace", why: "engine diagnostics run only under explicit performance, CRDT, or activation trace flags", gate: "TINE_PHASE_TRACE/TINE_CRDT_TRACE/TINE_ACTIVATION_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/hot_engine.rs", function: "prepare_transaction_core", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "directed-core-trace", why: "engine diagnostics run only under explicit performance, CRDT, or activation trace flags", gate: "TINE_PHASE_TRACE/TINE_CRDT_TRACE/TINE_ACTIVATION_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/hot_engine.rs", function: "record_validation_phase", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "numeric-trace", why: "validate_and_apply phase timing contains a phase index and a duration", gate: "TINE_PHASE_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/hot_engine.rs", function: "stage_ready_with_claim_source", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "directed-core-trace", why: "engine diagnostics run only under explicit performance, CRDT, or activation trace flags", gate: "TINE_PHASE_TRACE/TINE_CRDT_TRACE/TINE_ACTIVATION_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/hot_engine.rs", function: "validate_and_apply", macro_name: "eprintln", occurrences: &[0, 1, 2, 3], bucket: "b", class: "directed-core-trace", why: "engine diagnostics run only under explicit performance, CRDT, or activation trace flags", gate: "TINE_PHASE_TRACE/TINE_CRDT_TRACE/TINE_ACTIVATION_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/hot_engine.rs", function: "validate_manifested_projection_transition", macro_name: "eprintln", occurrences: &[0, 1, 2], bucket: "b", class: "directed-core-trace", why: "engine diagnostics run only under explicit performance, CRDT, or activation trace flags", gate: "TINE_PHASE_TRACE/TINE_CRDT_TRACE/TINE_ACTIVATION_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/hot_engine.rs", function: "validate_prospective_references", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "directed-core-trace", why: "engine diagnostics run only under explicit performance, CRDT, or activation trace flags", gate: "TINE_PHASE_TRACE/TINE_CRDT_TRACE/TINE_ACTIVATION_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/import.rs", function: "open_or_rebuild_clean_genesis_projection", macro_name: "eprintln", occurrences: &[0, 1], bucket: "a", class: "fixed-debug", why: "clean-genesis recovery reports one of two fixed states", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/oplog/local_journal_drain.rs", function: "resume_managed_local_journal_drain_with_parts_and_superseding_projection", macro_name: "eprintln", occurrences: &[0, 1, 2, 3], bucket: "b", class: "numeric-trace", why: "managed-local drain timings contain fixed labels and durations", gate: "TINE_PHASE_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/object_store.rs", function: "publish_immutable", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "enum-trace", why: "immutable publication reports only a fixed artifact class", gate: "TINE_PUBLISH_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/projection.rs", function: "execute_manifested_projection_work_located", macro_name: "eprintln", occurrences: &[0, 2, 3], bucket: "b", class: "directed-core-trace", why: "projection diagnostics are available only for an explicitly directed phase trace", gate: "TINE_PHASE_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/projection.rs", function: "execute_manifested_projection_work_located", macro_name: "eprintln", occurrences: &[1], bucket: "b", class: "directed-core-content", why: "this one DOES render target bytes as lossy UTF-8; it is graph content and stays behind the directed trace", gate: "TINE_PHASE_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/semantic.rs", function: "encode_validated", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "numeric-trace", why: "semantic snapshot diagnostic contains counts and encoded byte sizes", gate: "TINE_SEMANTIC_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/sqlite.rs", function: "apply_engine_owned_accepted_with_stats", macro_name: "eprintln", occurrences: &[0, 1, 2, 3, 4, 5, 6], bucket: "b", class: "directed-core-trace", why: "SQLite construction diagnostics run only under explicit trace flags", gate: "TINE_PHASE_TRACE/TINE_ACTIVATION_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/sqlite.rs", function: "rebuild_stream", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "directed-core-trace", why: "SQLite construction diagnostics run only under explicit trace flags", gate: "TINE_PHASE_TRACE/TINE_ACTIVATION_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/sqlite.rs", function: "trace_terminal_phase", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "directed-core-trace", why: "SQLite construction diagnostics run only under explicit trace flags", gate: "TINE_PHASE_TRACE/TINE_ACTIVATION_TRACE" },
    AllowedSite { file: "crates/tine-core/src/publish.rs", function: "publish_graph_documents_inner", macro_name: "eprintln", occurrences: &[0, 1], bucket: "a", class: "content-free-debug", why: "publication refusals report only a fixed shape or collision count", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "advance_clean_foreground_pending_commit", macro_name: "eprintln", occurrences: &[0], bucket: "a", class: "content-free-debug", why: "receipt, pending-projection, and conflict-resolution reports contain only counts or fixed states", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "advance_full_scan", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "directed-core-trace", why: "watcher trace is explicitly enabled for a directed investigation", gate: "TINE_CLEAN_WATCHER_TRACE" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "execute_editor_transaction", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "directed-core-detail", why: "foreground mutation detail is available only for explicitly enabled runtime debugging", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "execute_foreground_new_editor_transaction", macro_name: "eprintln", occurrences: &[0, 1], bucket: "b", class: "directed-core-detail", why: "foreground mutation detail is available only for explicitly enabled runtime debugging", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "finish_clean_foreground_editor_outcome", macro_name: "eprintln", occurrences: &[0], bucket: "a", class: "content-free-debug", why: "receipt, pending-projection, and conflict-resolution reports contain only counts or fixed states", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "open_clean_runtime_resources_with_progress", macro_name: "eprintln", occurrences: &[0, 1, 2, 3], bucket: "d", class: "content-free-error", why: "disposable checkpoint fallbacks carry typed store/decode errors describing batch ids, sealed kinds and sequences", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "open_clean_runtime_resources_with_progress", macro_name: "eprintln", occurrences: &[4], bucket: "a", class: "content-free-debug", why: "receipt, pending-projection, and conflict-resolution reports contain only counts or fixed states", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "phase", macro_name: "eprintln", occurrences: &[0], bucket: "a", class: "numeric-debug", why: "clean-open stage and counter reports contain fixed names and numeric measurements", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "qualify_clean_writer_lanes", macro_name: "eprintln", occurrences: &[0], bucket: "a", class: "content-free-debug", why: "CRDT writer-lane qualification reports a fixed disposition, an incarnation number and id, and a causal counter", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "report_counters", macro_name: "eprintln", occurrences: &[0], bucket: "a", class: "numeric-debug", why: "clean-open stage and counter reports contain fixed names and numeric measurements", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "resolve_pending_conflict", macro_name: "eprintln", occurrences: &[0, 1, 2], bucket: "a", class: "content-free-debug", why: "receipt, pending-projection, and conflict-resolution reports contain only counts or fixed states", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", function: "tick", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "numeric-trace", why: "actor tick report contains a fixed branch label, duration, and pending count", gate: "TINE_TICK_TRACE" },
    AllowedSite { file: "src-tauri/src/data_home.rs", function: "ensure_usable", macro_name: "eprintln", occurrences: &[0], bucket: "d", class: "fixed-terminal-failure", why: "fatal startup guidance is a fixed sentence plus the bounded ErrorKind token (I-9); the path and OS prose stay on diag", gate: "always-on fatal startup" },
    AllowedSite { file: "src-tauri/src/debug.rs", function: "debug_init", macro_name: "eprintln", occurrences: &[0, 1], bucket: "b", class: "directed-native-debug", why: "detailed native stderr, including the log path, is available only under the existing debug opt-in", gate: "debug_enabled" },
    AllowedSite { file: "src-tauri/src/debug.rs", function: "diag", macro_name: "eprintln", occurrences: &[0], bucket: "b", class: "directed-native-debug", why: "detailed native stderr, including the log path, is available only under the existing debug opt-in", gate: "debug_enabled" },
    AllowedSite { file: "src-tauri/src/debug.rs", function: "flight_init", macro_name: "eprintln", occurrences: &[0], bucket: "d", class: "content-free-error", why: "flight-recorder setup failure carries only a std::io::Error, whose Display never includes the path", gate: "always-on reviewed failure" },
];
/// Every print site with the line it currently sits on. The line is for the
/// failure message only — it is never compared, so it cannot make this test red.
fn print_sites() -> Vec<(PrintSite, usize)> {
    let root = repo_root();
    let files = production_source_files();
    let print_macro = Regex::new(r"\b(eprintln|println|dbg)!\s*[({\[]").unwrap();
    let mut sites = Vec::new();
    let mut counts: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    for file in files {
        let source = compiled_source(&file);
        let relative = relative_path(&root, &file);
        for found in print_macro.captures_iter(&source) {
            let whole = found.get(0).unwrap();
            if source[..whole.start()]
                .rsplit_once('\n')
                .map_or(&source[..whole.start()], |(_, line)| line)
                .trim_start()
                .starts_with("//")
            {
                continue;
            }
            let function = enclosing_fn(&source, whole.start());
            let macro_name = found[1].to_string();
            let occurrence = *counts
                .entry((relative.clone(), function.clone(), macro_name.clone()))
                .and_modify(|seen| *seen += 1)
                .or_insert(0);
            sites.push((
                PrintSite {
                    file: relative.clone(),
                    function,
                    macro_name,
                    occurrence,
                },
                line_of(&source, whole.start()),
            ));
        }
    }
    sites.sort();
    sites
}

#[test]
fn production_print_sites_equal_the_reviewed_content_free_census() {
    for entry in ALLOWLIST {
        assert!(!entry.class.is_empty(), "every print site needs a class");
        assert!(!entry.why.is_empty(), "every print site needs a reason");
        assert!(
            !entry.gate.is_empty(),
            "every print site needs an explicit gate"
        );
        assert!(
            matches!(entry.bucket, "a" | "b" | "d"),
            "I-5: `{}` is classified `{}`. Class (c) — always-on plus a variable that can carry \
             user content — is ZERO, and stays zero. Do not classify a site into (c); fix it: \
             report the fixed failure family always-on and put the raw value behind \
             `runtime_debug_diagnostics_enabled()` (exemplar: \
             `direct_projection.rs::report_projection_failure`), or emit a fixed-shape event \
             (exemplar: `src-tauri/src/debug.rs::record_fixed_event`). Only a–b–d are valid.",
            entry.file,
            entry.bucket
        );
    }
    let mut expected = ALLOWLIST
        .iter()
        .flat_map(|entry| {
            entry.occurrences.iter().map(|occurrence| PrintSite {
                file: entry.file.to_string(),
                function: entry.function.to_string(),
                macro_name: entry.macro_name.to_string(),
                occurrence: *occurrence,
            })
        })
        .collect::<Vec<_>>();
    expected.sort();
    // A row must name ONE site. Two rows claiming the same occurrence of the
    // same function would let either of them supply the classification, which
    // is the ambiguity the line pin had.
    let mut unique = expected.clone();
    unique.dedup();
    assert_eq!(
        unique.len(),
        expected.len(),
        "I-5: two ALLOWLIST rows classify the same print site"
    );
    assert_eq!(expected.len(), RUST_PRINT_SITE_COUNT);

    let actual = print_sites();
    let repair = "I-5: the production print-site census changed. \
         A print site was ADDED to, REMOVED from, or MOVED BETWEEN functions — each is a real \
         change to what a shipped binary can emit. Remove user content and use a fixed-shape \
         event (src-tauri — `debug.rs::record_fixed_event` and its typed callers such as \
         `record_storage_transition` are the exemplar) or a content-free flag-gated line (core), \
         then classify the exact site by hand in ALLOWLIST and adjust RUST_PRINT_SITE_COUNT. \
         Rows are anchored to a function and an occurrence within it, NOT to a line number, so \
         this does not fire merely because lines moved — there is nothing to re-anchor.";
    let added = actual
        .iter()
        .filter(|(site, _)| !expected.contains(site))
        .map(|(site, line)| format!("{} (now at {}:{line})", site.identity(), site.file))
        .collect::<Vec<_>>();
    assert!(added.is_empty(), "{repair}\nnot in the census: {added:#?}");
    let present = actual
        .iter()
        .map(|(site, _)| site.clone())
        .collect::<Vec<_>>();
    let removed = expected
        .iter()
        .filter(|site| !present.contains(site))
        .map(PrintSite::identity)
        .collect::<Vec<_>>();
    assert!(
        removed.is_empty(),
        "{repair}\ncensused but no longer present: {removed:#?}"
    );
}

/// Name of the innermost preceding `fn` declaration — enough to say WHICH
/// function a scanned site lives in, which is the unit I-12 talks about.
fn enclosing_fn(source: &str, offset: usize) -> String {
    let declaration =
        Regex::new(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|unsafe\s+|const\s+)*fn\s+(\w+)")
            .unwrap();
    declaration
        .captures_iter(&source[..offset])
        .last()
        .map_or_else(|| "<file scope>".to_owned(), |found| found[1].to_string())
}

/// I-12: "are runtime debug diagnostics on?" has exactly one answer.
///
/// Two separate counts, because they are two separate ways to grow a second
/// answer. The process-level opt-in (`TINE_DEBUG=1` / `--debug`) is PARSED in
/// exactly one function, at startup; the resulting flag is READ in exactly one
/// function, `runtime_debug_diagnostics_enabled`, which every other diagnostic
/// in core and src-tauri delegates to.
#[test]
fn exactly_one_function_reads_the_debug_diagnostics_flag() {
    let root = repo_root();
    let opt_in = Regex::new(r#""TINE_DEBUG"|"--debug""#).unwrap();
    let flag_read = Regex::new(r"RUNTIME_DEBUG_DIAGNOSTICS\s*\.\s*load\s*\(").unwrap();
    let mut parses = Vec::new();
    let mut reads = Vec::new();
    for file in production_source_files() {
        let source = compiled_source(&file);
        let relative = relative_path(&root, &file);
        // Deliberately (file, function) and NOT (file, line): a line-anchored
        // guard reddens on every unrelated edit above it, which trains readers
        // to re-anchor without reading. The function is the unit I-12 means.
        for found in opt_in.find_iter(&source) {
            parses.push(format!(
                "{relative}::{}",
                enclosing_fn(&source, found.start())
            ));
        }
        for found in flag_read.find_iter(&source) {
            reads.push(format!(
                "{relative}::{}",
                enclosing_fn(&source, found.start())
            ));
        }
    }
    let repair = "I-12: \"are runtime debug diagnostics on?\" must have ONE producer. \
         src-tauri's `debug_opt_in_requested` parses `TINE_DEBUG` / `--debug` once at startup \
         and hands the answer to `tine_core::sync_runtime::set_runtime_debug_diagnostics`; \
         every diagnostic then asks the front door \
         `tine_core::sync_runtime::runtime_debug_diagnostics_enabled()` \
         (src-tauri's `debug_enabled()` is a thin delegate to it, and is the exemplar to \
         imitate). Do not re-read the environment in a second function: a per-crate parse \
         cannot be steered by the host process and drifts silently.";
    assert_eq!(
        parses,
        vec![
            "src-tauri/src/debug.rs::debug_opt_in_requested".to_owned(),
            "src-tauri/src/debug.rs::debug_opt_in_requested".to_owned(),
        ],
        "{repair}"
    );
    assert_eq!(
        reads,
        vec!["crates/tine-core/src/sync_runtime.rs::runtime_debug_diagnostics_enabled".to_owned()],
        "{repair}"
    );
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn corpus_page_names(root: &Path, names: &mut Vec<String>) {
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            corpus_page_names(&entry.path(), names);
            continue;
        }
        if !entry.file_type().unwrap().is_file()
            || !matches!(
                entry.path().extension().and_then(|value| value.to_str()),
                Some("md" | "markdown" | "org")
            )
        {
            continue;
        }
        if let Some(name) = entry.path().file_stem().and_then(|value| value.to_str()) {
            if !name.is_empty() {
                names.push(name.to_owned());
            }
        }
    }
}

#[test]
#[ignore = "child process for the real-corpus stderr probe"]
fn real_corpus_open_save_publish_child() {
    let source = PathBuf::from(std::env::var_os("TINE_REAL_GRAPH").expect("TINE_REAL_GRAPH"));
    let scratch = std::env::temp_dir().join(format!(
        "tine-i5-content-out-of-logs-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    copy_tree(&source, &scratch);
    fs::create_dir_all(scratch.join("pages")).unwrap();
    fs::write(
        scratch.join("pages/I5 Diagnostics Probe.md"),
        "public:: true\n\n- fixed-shape diagnostics probe\n",
    )
    .unwrap();

    let graph = tine_core::model::Graph::open_checked(&scratch).unwrap();
    graph.warm_cache();
    let mut page = graph
        .load_by_path("pages/I5 Diagnostics Probe.md")
        .unwrap()
        .unwrap();
    page.blocks[0].raw.push_str(" updated");
    let baseline = page.rev.clone();
    graph.save_page(&page, baseline.as_deref()).unwrap();
    let (_, published) = graph.publish_html().unwrap();
    assert!(published > 0, "real-corpus publish produced no public page");
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
#[ignore = "manual real-corpus gate: set TINE_REAL_GRAPH"]
fn real_corpus_open_save_publish_emits_no_page_name_with_debug_disabled() {
    let source = PathBuf::from(std::env::var_os("TINE_REAL_GRAPH").expect("TINE_REAL_GRAPH"));
    let mut names = Vec::new();
    corpus_page_names(&source, &mut names);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "real_corpus_open_save_publish_child",
            "--nocapture",
        ])
        .env("TINE_REAL_GRAPH", &source)
        .env_remove("TINE_DEBUG")
        .output()
        .unwrap();
    assert!(output.status.success(), "real-corpus child failed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let matches = names
        .iter()
        .filter(|name| stderr.contains(name.as_str()))
        .count();
    println!(
        "I5_REAL_CORPUS files={} stderr_bytes={} page_name_matches={matches}",
        names.len(),
        stderr.len()
    );
    assert_eq!(
        matches, 0,
        "captured stderr contained {matches} corpus page-name matches"
    );
}

/// I-11: a directed trace flag is a contract row, not a free-text gate.
///
/// The class-(b) `gate:` fields above and the class-(b) paragraph of
/// `docs/contracts/diagnostics.md` named the same flags in two places with
/// nothing comparing them, so either could drift silently — a flag could be
/// retired from the contract and still gate a live print site, or a new
/// `TINE_*_TRACE` channel could be added with no contract row at all. Both
/// directions are pinned here.
///
/// `docs/contracts/diagnostics.md` is the exemplar to follow when adding a
/// channel: name the flag in that paragraph first, then classify the site.
///
/// Scope: `crates/tine-core/src` only. src-tauri's directed channel is
/// `debug_enabled()` behind `TINE_DEBUG`, which has its own producer pin in
/// `exactly_one_function_reads_the_debug_diagnostics_flag`; it reads no
/// `_TRACE` flag, and this packet does not touch that tree.
#[test]
fn directed_trace_flags_are_the_same_set_in_the_contract_and_in_the_census() {
    let repair = "I-11: a directed trace flag is a contract row, not a free-text gate. \
         Every `TINE_*_TRACE` channel is named in the class (b) paragraph of \
         `docs/contracts/diagnostics.md` (the exemplar) AND appears in the `gate:` field of \
         the class (b) ALLOWLIST row for the site it gates. Add the contract row first.";
    let flag = Regex::new(r"TINE_[A-Z0-9_]*_TRACE").unwrap();

    let contract = fs::read_to_string(repo_root().join("docs/contracts/diagnostics.md")).unwrap();
    let paragraph = contract
        .split_once("The retained class (b) channels are named")
        .expect("diagnostics.md must carry the class (b) channel paragraph")
        .1
        .split_once("\n\n")
        .expect("the class (b) channel paragraph must end")
        .0;
    let mut contract_flags = flag
        .find_iter(paragraph)
        .map(|found| found.as_str().to_owned())
        .collect::<Vec<_>>();
    contract_flags.sort();
    contract_flags.dedup();
    assert!(
        paragraph.contains("TINE_DEBUG"),
        "{repair} The process-level `TINE_DEBUG` opt-in stays named in the same paragraph; \
         it is not a directed trace channel and has its own producer pin."
    );

    let mut census_flags = ALLOWLIST
        .iter()
        .filter(|entry| entry.bucket == "b")
        .flat_map(|entry| {
            flag.find_iter(entry.gate)
                .map(|found| found.as_str().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    census_flags.sort();
    census_flags.dedup();
    assert_eq!(census_flags, contract_flags, "{repair}");

    let root = repo_root();
    let core = root.join("crates/tine-core/src");
    let read = Regex::new(r#"env::var(?:_os)?\s*\(\s*"(TINE_[A-Z0-9_]+)""#).unwrap();
    let mut unpinned = Vec::new();
    for file in production_source_files() {
        if !file.starts_with(&core) {
            continue;
        }
        let source = compiled_source(&file);
        for found in read.captures_iter(&source) {
            let name = found[1].to_string();
            if !name.ends_with("_TRACE") || contract_flags.contains(&name) {
                continue;
            }
            unpinned.push(format!(
                "{}:{} {name}",
                relative_path(&root, &file),
                line_of(&source, found.get(0).unwrap().start())
            ));
        }
    }
    assert!(unpinned.is_empty(), "{repair} Unpinned: {unpinned:?}");
}
