//! A thread-local that owns a thread must drop it off the loader lock.
//!
//! Windows runs thread-local destructors under the loader lock. A value whose
//! drop joins a thread (a page host joins its driver, a store joins its
//! watcher) then waits for a thread exit that needs that same lock, and the
//! whole test binary hangs (Windows CI, 2026-11-17). `ThreadSlot` drops what it
//! holds on a fresh thread instead. This scan makes the rule fail on every
//! platform, not only on Windows, so a new thread-local cannot reintroduce it.

use regex::Regex;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Types whose own `Drop` joins a thread. Every struct that holds one of them,
/// directly or through another such struct, is derived from these below.
const JOINS_ON_DROP: [&str; 2] = ["PageHost", "Store"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn idents(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|s| !s.is_empty())
}

/// Struct names whose fields reach a type in `JOINS_ON_DROP`.
fn thread_owning(sources: &[String]) -> BTreeSet<String> {
    let structs =
        Regex::new(r"(?s)\bstruct\s+(\w+)\s*(?:<[^>{(;]*>)?\s*(\{[^{}]*\}|\([^;{}]*\);)").unwrap();
    let defs: Vec<(String, String)> = sources
        .iter()
        .flat_map(|text| {
            structs
                .captures_iter(text)
                .map(|c| (c[1].to_owned(), c[2].to_owned()))
        })
        .collect();
    let mut owning: BTreeSet<String> = JOINS_ON_DROP.iter().map(|s| (*s).to_owned()).collect();
    loop {
        let before = owning.len();
        for (name, body) in &defs {
            if idents(body).any(|id| owning.contains(id)) {
                owning.insert(name.clone());
            }
        }
        if owning.len() == before {
            return owning;
        }
    }
}

/// `(file, static name)` for every thread-local static that holds a
/// thread-owning type without a `ThreadSlot`.
fn unslotted(files: &[(String, String)], owning: &BTreeSet<String>) -> Vec<(String, String)> {
    let statics = Regex::new(r"(?s)\bstatic\s+(\w+)\s*:(.*?)=").unwrap();
    let mut found = Vec::new();
    for (path, text) in files {
        for block in text.split("thread_local!").skip(1) {
            let Some(open) = block.find(['{', '(']) else {
                continue;
            };
            let mut depth = 0usize;
            let mut end = block.len();
            for (i, c) in block[open..].char_indices() {
                match c {
                    '{' | '(' => depth += 1,
                    '}' | ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            for c in statics.captures_iter(&block[open..end]) {
                let ty = &c[2];
                if idents(ty).any(|id| owning.contains(id))
                    && !idents(ty).any(|id| id == "ThreadSlot")
                {
                    found.push((path.clone(), c[1].to_owned()));
                }
            }
        }
    }
    found
}

#[test]
fn thread_locals_that_own_a_thread_drop_it_off_the_loader_lock() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut paths = Vec::new();
    for dir in ["crates", "src-tauri/src"] {
        rust_files(&root.join(dir), &mut paths);
    }
    // Comments are dropped, so a word in a doc comment is not a field type.
    let comment = Regex::new(r"//[^\n]*").unwrap();
    let files: Vec<(String, String)> = paths
        .iter()
        // This file's own test fixtures spell out the forbidden form.
        .filter(|p| !p.ends_with("thread_local_hosts_guard.rs"))
        .map(|p| {
            let text = std::fs::read_to_string(p).unwrap();
            (
                p.strip_prefix(&root).unwrap_or(p).display().to_string(),
                comment.replace_all(&text, "").into_owned(),
            )
        })
        .collect();
    assert!(
        files
            .iter()
            .any(|(p, _)| p.ends_with("test_host_writer.rs")),
        "scan found the sources"
    );
    // Struct names are resolved within one crate: two crates may each have a `Session`.
    let crate_of = |path: &str| {
        path.split('/')
            .take(if path.starts_with("crates/") { 2 } else { 1 })
            .collect::<Vec<_>>()
            .join("/")
    };
    let crates: BTreeSet<String> = files.iter().map(|(p, _)| crate_of(p)).collect();
    let mut found = Vec::new();
    for name in &crates {
        let in_crate: Vec<(String, String)> = files
            .iter()
            .filter(|(p, _)| &crate_of(p) == name)
            .cloned()
            .collect();
        let owning = thread_owning(&in_crate.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>());
        if name == "crates/tine-store" {
            assert!(
                owning.contains("HostWriter"),
                "derivation reaches the test host: {owning:?}"
            );
        }
        found.extend(unslotted(&in_crate, &owning));
    }
    assert!(
        found.is_empty(),
        "thread-locals holding a thread-owning value without ThreadSlot (they hang Windows test runs): {found:?}"
    );
}

#[test]
fn the_scan_flags_a_host_held_without_a_slot() {
    let sources =
        vec!["struct Running { host: PageHost }\nstruct HostWriter(Arc<Running>);".to_owned()];
    let owning = thread_owning(&sources);
    let file = |body: &str| vec![("x.rs".to_owned(), format!("thread_local! {{\n{body}\n}}"))];
    let bare = "static CURRENT: RefCell<Option<HostWriter>> = const { RefCell::new(None) };";
    let slotted = "static CURRENT: RefCell<ThreadSlot<HostWriter>> = const { RefCell::new(ThreadSlot(None)) };";
    let counter = "static COUNT: Cell<usize> = const { Cell::new(0) };";
    assert_eq!(
        unslotted(&file(bare), &owning),
        vec![("x.rs".to_owned(), "CURRENT".to_owned())]
    );
    assert!(unslotted(&file(slotted), &owning).is_empty());
    assert!(unslotted(&file(counter), &owning).is_empty());
}
