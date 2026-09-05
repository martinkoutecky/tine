//! The corpus-walking helpers both query-engine example binaries share
//! (`query-walk-dump`, the O8 parity exporter, and `query-gate1-dump`, gate 1's
//! walk side).
//!
//! Two copies of a macro scanner would drift, and a drift here is silent: the
//! gate would compare a different set of queries than the parity export. This
//! module is the one producer (D-4). It lives under `examples/support/` so
//! Cargo does not try to build it as an example of its own.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Every `{{query …}}` / `{{tine-query …}}` macro argument in one file, in
/// source order. Brace-balanced so a trailing options map `{:title …}` stays
/// inside the macro.
pub fn macro_args(text: &str, name: &str) -> Vec<String> {
    let opener = format!("{{{{{name}");
    let bytes: Vec<char> = text.chars().collect();
    let opener_chars: Vec<char> = opener.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + opener_chars.len() <= bytes.len() {
        if bytes[i..i + opener_chars.len()] != opener_chars[..] {
            i += 1;
            continue;
        }
        let after = i + opener_chars.len();
        // `{{query}}` and `{{query …}}`; `{{query-foo}}` is a different macro.
        match bytes.get(after) {
            Some(' ') | Some('\t') | Some('\n') => {}
            Some('}') => {
                i = after;
                continue;
            }
            _ => {
                i += 1;
                continue;
            }
        }
        let mut depth = 1usize;
        let mut j = after;
        let mut arg = String::new();
        while j < bytes.len() {
            if bytes[j] == '{' && bytes.get(j + 1) == Some(&'{') {
                depth += 1;
                arg.push('{');
                arg.push('{');
                j += 2;
                continue;
            }
            if bytes[j] == '}' && bytes.get(j + 1) == Some(&'}') {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                arg.push('}');
                arg.push('}');
                j += 2;
                continue;
            }
            arg.push(bytes[j]);
            j += 1;
        }
        if depth == 0 {
            out.push(arg.trim().to_string());
        }
        i = j.max(after);
    }
    out
}

pub fn graph_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["pages", "journals"] {
        let dir = root.join(dir);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if matches!(ext.as_deref(), Some("md") | Some("org") | Some("markdown")) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// A single Markdown/Org file (the kitchen-sink fixture) materialized as a
/// one-page graph in a temp dir so it can be walked like the others. Returns
/// the directory to walk and the scratch dir to remove afterwards.
pub fn materialize_single_file(root: &Path) -> (PathBuf, Option<PathBuf>) {
    if !root.is_file() {
        return (root.to_path_buf(), None);
    }
    let stem = root
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("page")
        .to_string();
    let dir = std::env::temp_dir().join(format!("query-corpus-{stem}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("pages")).expect("scratch graph");
    let ext = root.extension().and_then(|e| e.to_str()).unwrap_or("md");
    std::fs::copy(root, dir.join("pages").join(format!("{stem}.{ext}"))).expect("scratch page");
    (dir.clone(), Some(dir))
}
