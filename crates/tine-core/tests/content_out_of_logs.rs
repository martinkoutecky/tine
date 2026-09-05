#[path = "support/production_source.rs"]
mod production_source;

use production_source::{
    compiled_source, line_of, production_source_files, relative_path, repo_root,
};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct PrintSite {
    file: String,
    line: usize,
    macro_name: String,
}

#[derive(Clone, Copy)]
struct AllowedSite {
    file: &'static str,
    lines: &'static [usize],
    macro_name: &'static str,
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
const RUST_PRINT_SITE_COUNT: usize = 74;
const ALLOWLIST: &[AllowedSite] = &[
    AllowedSite { file: "crates/tine-core/src/concord_ledger.rs", lines: &[238], macro_name: "eprintln", bucket: "d", class: "content-free-error", why: "best-effort ledger update failure carries only a std::io::Error, whose Display never includes the path", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/direct_projection.rs", lines: &[999], macro_name: "eprintln", bucket: "d", class: "fixed-family-report", why: "report_projection_failure's always-on line is the fixed failure family and no error value", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/direct_projection.rs", lines: &[1001], macro_name: "eprintln", bucket: "b", class: "directed-core-detail", why: "report_projection_failure's detail line repeats the family with the raw error, for a directed investigation", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/direct_projection.rs", lines: &[1012], macro_name: "eprintln", bucket: "d", class: "content-free-error", why: "projection directory creation carries only a std::io::Error, whose Display never includes the path", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/direct_projection.rs", lines: &[1030], macro_name: "eprintln", bucket: "d", class: "content-free-error", why: "projection lease acquisition carries only a std::io::Error, whose Display never includes the path", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/model.rs", lines: &[14645], macro_name: "eprintln", bucket: "d", class: "numeric-shape", why: "isolated search worker panic reports only a worker number", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/model.rs", lines: &[20335, 23219], macro_name: "eprintln", bucket: "a", class: "content-free-debug", why: "reconcile and isolated-parse failures contain no path, title, content, or raw error", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/model.rs", lines: &[22086, 22275], macro_name: "eprintln", bucket: "a", class: "fixed-debug", why: "Guide-page refusal messages are fixed literals", gate: "cfg(debug_assertions)" },
    AllowedSite { file: "crates/tine-core/src/oplog/batch.rs", lines: &[205], macro_name: "eprintln", bucket: "b", class: "numeric-trace", why: "batch composition contains public enum kinds and numeric sizes only", gate: "TINE_BATCH_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/checkpoint_generation.rs", lines: &[999], macro_name: "eprintln", bucket: "d", class: "content-free-error", why: "checkpoint writer thread spawn carries only a std::io::Error", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/oplog/checkpoint_generation.rs", lines: &[1046], macro_name: "eprintln", bucket: "d", class: "content-free-error", why: "publish_capture's error is a bounded literal or an object-store error over digest-named private blobs", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/oplog/hot_engine.rs", lines: &[8811], macro_name: "eprintln", bucket: "b", class: "numeric-trace", why: "validate_and_apply phase timing contains a phase index and a duration", gate: "TINE_PHASE_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/hot_engine.rs", lines: &[10864, 14409, 15425, 15453, 15632, 18174, 18203, 18237, 18269, 19126, 19152, 19290, 19309, 21695, 24353], macro_name: "eprintln", bucket: "b", class: "directed-core-trace", why: "engine diagnostics run only under explicit performance, CRDT, or activation trace flags", gate: "TINE_PHASE_TRACE/TINE_CRDT_TRACE/TINE_ACTIVATION_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/import.rs", lines: &[1711, 1723], macro_name: "eprintln", bucket: "a", class: "fixed-debug", why: "clean-genesis recovery reports one of two fixed states", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/oplog/local_journal_drain.rs", lines: &[846, 873, 889, 914], macro_name: "eprintln", bucket: "b", class: "numeric-trace", why: "managed-local drain timings contain fixed labels and durations", gate: "TINE_PHASE_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/object_store.rs", lines: &[1888], macro_name: "eprintln", bucket: "b", class: "enum-trace", why: "immutable publication reports only a fixed artifact class", gate: "TINE_PUBLISH_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/projection.rs", lines: &[2870, 3165, 3209], macro_name: "eprintln", bucket: "b", class: "directed-core-trace", why: "projection diagnostics are available only for an explicitly directed phase trace", gate: "TINE_PHASE_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/projection.rs", lines: &[2975], macro_name: "eprintln", bucket: "b", class: "directed-core-content", why: "this one DOES render target bytes as lossy UTF-8; it is graph content and stays behind the directed trace", gate: "TINE_PHASE_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/semantic.rs", lines: &[935], macro_name: "eprintln", bucket: "b", class: "numeric-trace", why: "semantic snapshot diagnostic contains counts and encoded byte sizes", gate: "TINE_SEMANTIC_TRACE" },
    AllowedSite { file: "crates/tine-core/src/oplog/sqlite.rs", lines: &[1893, 4501, 4505, 4512, 4525, 4537, 4545, 4557, 5317], macro_name: "eprintln", bucket: "b", class: "directed-core-trace", why: "SQLite construction diagnostics run only under explicit trace flags", gate: "TINE_PHASE_TRACE/TINE_TERMINAL_TRACE" },
    AllowedSite { file: "crates/tine-core/src/publish.rs", lines: &[4400, 4430], macro_name: "eprintln", bucket: "a", class: "content-free-debug", why: "publication refusals report only a fixed shape or collision count", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", lines: &[6416], macro_name: "eprintln", bucket: "b", class: "directed-core-trace", why: "watcher trace is explicitly enabled for a directed investigation", gate: "TINE_CLEAN_WATCHER_TRACE" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", lines: &[7191, 7209], macro_name: "eprintln", bucket: "a", class: "numeric-debug", why: "clean-open stage and counter reports contain fixed names and numeric measurements", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", lines: &[7381, 7385, 7411, 7423], macro_name: "eprintln", bucket: "d", class: "content-free-error", why: "disposable checkpoint fallbacks carry typed store/decode errors describing batch ids, sealed kinds and sequences", gate: "always-on reviewed failure" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", lines: &[7555, 21526, 21787, 22637, 22677, 22709], macro_name: "eprintln", bucket: "a", class: "content-free-debug", why: "receipt, pending-projection, and conflict-resolution reports contain only counts or fixed states", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", lines: &[21197, 21288, 21354], macro_name: "eprintln", bucket: "b", class: "directed-core-detail", why: "foreground mutation detail is available only for explicitly enabled runtime debugging", gate: "runtime_debug_diagnostics_enabled" },
    AllowedSite { file: "crates/tine-core/src/sync_runtime.rs", lines: &[22169], macro_name: "eprintln", bucket: "b", class: "numeric-trace", why: "actor tick report contains a fixed branch label, duration, and pending count", gate: "TINE_TICK_TRACE" },
    AllowedSite { file: "src-tauri/src/data_home.rs", lines: &[128], macro_name: "eprintln", bucket: "d", class: "fixed-terminal-failure", why: "fatal startup guidance is a fixed sentence plus the bounded ErrorKind token (I-9); the path and OS prose stay on diag", gate: "always-on fatal startup" },
    AllowedSite { file: "src-tauri/src/debug.rs", lines: &[86, 90, 104], macro_name: "eprintln", bucket: "b", class: "directed-native-debug", why: "detailed native stderr, including the log path, is available only under the existing debug opt-in", gate: "debug_enabled" },
    AllowedSite { file: "src-tauri/src/debug.rs", lines: &[383], macro_name: "eprintln", bucket: "d", class: "content-free-error", why: "flight-recorder setup failure carries only a std::io::Error, whose Display never includes the path", gate: "always-on reviewed failure" },
];

fn print_sites() -> Vec<PrintSite> {
    let root = repo_root();
    let files = production_source_files();
    let print_macro = Regex::new(r"\b(eprintln|println|dbg)!\s*[({\[]").unwrap();
    let mut sites = Vec::new();
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
            sites.push(PrintSite {
                file: relative.clone(),
                line: line_of(&source, whole.start()),
                macro_name: found[1].to_string(),
            });
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
    let actual = print_sites();
    let mut expected = ALLOWLIST
        .iter()
        .flat_map(|entry| {
            entry.lines.iter().map(|line| PrintSite {
                file: entry.file.to_string(),
                line: *line,
                macro_name: entry.macro_name.to_string(),
            })
        })
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(expected.len(), RUST_PRINT_SITE_COUNT);
    assert_eq!(
        actual, expected,
        "I-5: production print-site census changed. \
         If a print site was ADDED or REMOVED: remove user content and use a fixed-shape event \
         (src-tauri — `debug.rs::record_fixed_event` and its typed callers such as \
         `record_storage_transition` are the exemplar) or a content-free flag-gated line (core), \
         then classify the exact site by hand in ALLOWLIST. \
         If the (file, macro) multiset is UNCHANGED and only line numbers moved, this is pure \
         drift from an edit above a print site: run `node scripts/reanchor-print-census.mjs`, \
         which re-anchors line numbers only and refuses to bless an added or removed site."
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
