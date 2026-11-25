//! One answer to "is this one of Tine's temporary names": `src/atomic_file.rs`
//! (`parse_temp_name`, `is_fixed_temp_name`, `is_temp_name`) owns the grammar of
//! the temps it makes. A second recogniser elsewhere drifts: Git autocommit's own
//! reader missed every tagged temp (`.new`, `.capture`, …) and the fixed-form
//! `.tine-*.tmp`, so a new tag reached the graph watcher and not Git. This guard
//! fails when production code outside the owner reads a `.tmp` suffix itself,
//! unless the line is listed below with the different question it answers.
use std::fs;
use std::path::Path;

use crate::c3y_derived_names_guard::{production_lines, sources};

const OWNER: &str = "crates/tine-store/src/atomic_file.rs";

/// Ways code recognises a temp file by its `.tmp` suffix.
const RECOGNISERS: &[&str] = &[
    "strip_suffix(\".tmp\")",
    "strip_suffix(b\".tmp\")",
    "ends_with(\".tmp\")",
    "ends_with(b\".tmp\")",
];

/// `(file, line text, question)`: lines that ask a different question than
/// "is this one of Tine's temp names", so they do not go through the owner.
const OTHER_QUESTIONS: &[(&str, &str, &str)] = &[
    (
        "crates/tine-store/src/page_host/model_fs.rs",
        ".filter(|key| key.starts_with(CUSTODY) && key.ends_with(\".tmp\"))",
        "any unpublished file in the private custody folder, whoever wrote it",
    ),
    (
        "crates/tine-store/src/page_host/production.rs",
        "|| entry.file_name().to_string_lossy().ends_with(\".tmp\")",
        "a draft-folder entry that is no draft (the private drafts folder)",
    ),
    (
        "crates/tine-store/src/page_host/production.rs",
        "if name.ends_with(\".tmp\") {",
        "any unpublished file in the private custody folder, whoever wrote it",
    ),
    (
        "src-tauri/src/owner/git_run.rs",
        "&& (name.ends_with(\".tmp\") || name.ends_with(\".tmp.lock\"))",
        "Git index scratch (`tine-index-*`) inside `.git`, not a graph file",
    ),
];

#[test]
fn only_atomic_file_recognises_tine_temp_names() {
    let _integration_guard = crate::integration_lock();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for crate_dir in fs::read_dir(repo.join("crates")).unwrap().flatten() {
        let src = crate_dir.path().join("src");
        if src.is_dir() {
            sources(&src, &mut files);
        }
    }
    sources(&repo.join("src-tauri/src"), &mut files);
    assert!(
        files.len() > 100,
        "guard scanned too few files: {}",
        files.len()
    );
    let mut owner_seen = false;
    let mut listed_seen = vec![false; OTHER_QUESTIONS.len()];
    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&repo)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&file).unwrap();
        for (line_no, line) in production_lines(&text) {
            let code = line.split("//").next().unwrap_or("").trim();
            if !RECOGNISERS.iter().any(|needle| code.contains(needle)) {
                continue;
            }
            if rel == OWNER {
                owner_seen = true;
                continue;
            }
            match OTHER_QUESTIONS
                .iter()
                .position(|(path, listed, _)| *path == rel && *listed == code)
            {
                Some(index) => listed_seen[index] = true,
                None => violations.push(format!("{rel}:{line_no}: {code}")),
            }
        }
    }
    assert!(
        owner_seen,
        "the guard no longer sees the owner's recogniser in {OWNER}"
    );
    let stale: Vec<_> = OTHER_QUESTIONS
        .iter()
        .zip(&listed_seen)
        .filter(|(_, seen)| !**seen)
        .map(|((path, line, _), _)| format!("{path}: {line}"))
        .collect();
    assert!(
        stale.is_empty(),
        "listed lines no longer exist:\n{}",
        stale.join("\n")
    );
    assert!(
        violations.is_empty(),
        "a Tine temp name is recognised outside {OWNER}; call `atomic_file::is_temp_name` \
         or `parse_temp_name` (the app reaches them through `device_io::atomic_file`), or list the line in OTHER_QUESTIONS \
         with the different question it answers. Found:\n{}",
        violations.join("\n")
    );
}
