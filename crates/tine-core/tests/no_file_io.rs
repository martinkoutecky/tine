//! og batch 1 (`tine-agents/og/batches/01-arrival.md`, budget B): tine-core is
//! pure — parsing, serialization and evaluation over values. Reading or writing
//! the graph belongs to tine-store, the one storage module. A match below means
//! file I/O crept back into tine-core: move that code into tine-store and pass
//! tine-core the bytes or values instead.

use std::path::Path;

const FORBIDDEN: &[&str] = &[
    "std::fs",
    "fs::",
    "File::",
    "OpenOptions",
    "read_dir(",
    "canonicalize(",
    "symlink_metadata(",
    ".metadata(",
    ".exists()",
    ".is_file()",
    ".is_dir()",
    "std::process",
    "std::net",
];

/// Crates whose only use is touching the filesystem or the OS.
const FORBIDDEN_DEPS: &[&str] = &["cap-std", "same-file", "libc", "windows-sys", "notify"];

/// `pat` occurs in `code` not as the tail of a longer identifier
/// (`refs::` must not match `fs::`).
fn contains_token(code: &str, pat: &str) -> bool {
    code.match_indices(pat).any(|(i, _)| {
        !pat.starts_with(|c: char| c.is_alphanumeric() || c == '_')
            || !code[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_')
    })
}

fn scan(dir: &Path, hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path, hits);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                for pat in FORBIDDEN {
                    if contains_token(code, pat) {
                        hits.push(format!(
                            "{}:{}: `{pat}` in `{}`",
                            path.display(),
                            n + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn tine_core_source_does_no_file_io() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    scan(&root.join("src"), &mut hits);
    assert!(
        hits.is_empty(),
        "tine-core must stay free of file I/O (01-arrival.md B); move this into tine-store:\n{}",
        hits.join("\n")
    );
}

#[test]
fn tine_core_has_no_os_dependencies() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let mut section = String::new();
    let mut hits = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        if section.contains("dev-dependencies") || !section.contains("dependencies") {
            continue;
        }
        if let Some(name) = line.split('=').next().map(str::trim) {
            if FORBIDDEN_DEPS.contains(&name) {
                hits.push(format!("{section} {name}"));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "tine-core depends on an OS/filesystem crate (01-arrival.md B); that code belongs in tine-store: {hits:?}"
    );
}
