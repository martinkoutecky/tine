//! Corpus parity exporter for the query-engine campaign (SPEC §8, dossier O8).
//!
//! Runs every `{{query …}}` found in the given graph directories through the
//! in-memory walk and prints one deterministic line per query:
//!
//! ```text
//! <graph>\t<page>\t<query-hash>\t<anchor>\t<total>\t<exceeded>\t<id,id,…>
//! ```
//!
//! Only identifiers are printed — never block or query text — so the output of a
//! private graph can be diffed and pasted into a receipt. The query text itself
//! is reduced to a stable 16-hex-digit FNV-1a hash.
//!
//! Usage:
//! `cargo run -q -p tine-core --example query-walk-dump -- <graph-dir>…`
//!
//! A graph directory that does not exist is reported as `# skipped <label>` on
//! stdout rather than failing, so the same command line works with and without
//! the anonymized graph.

use std::path::{Path, PathBuf};

use tine_core::query::run_query_bounded;
use tine_core::Graph;

fn fnv1a(bytes: &[u8]) -> u64 {
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
fn macro_args(text: &str, name: &str) -> Vec<String> {
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

fn graph_files(root: &Path) -> Vec<PathBuf> {
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: query-walk-dump <graph-dir>…");
        std::process::exit(2);
    }
    for arg in args {
        let root = PathBuf::from(&arg);
        let label = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(arg.as_str())
            .to_string();
        // A single Markdown/Org file (the kitchen-sink fixture) is materialized
        // as a one-page graph in a temp dir so it can be walked like the others.
        let mut scratch: Option<PathBuf> = None;
        let root = if root.is_file() {
            let stem = root
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("page")
                .to_string();
            let dir = std::env::temp_dir().join(format!("query-walk-dump-{stem}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("pages")).expect("scratch graph");
            let ext = root.extension().and_then(|e| e.to_str()).unwrap_or("md");
            std::fs::copy(&root, dir.join("pages").join(format!("{stem}.{ext}")))
                .expect("scratch page");
            scratch = Some(dir.clone());
            dir
        } else {
            root
        };
        if !root.is_dir() {
            println!("# skipped {label} (absent)");
            continue;
        }
        let graph = Graph::open(&root);
        let mut rows: Vec<String> = Vec::new();
        for file in graph_files(&root) {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let page = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let mut queries = macro_args(&text, "query");
            queries.extend(macro_args(&text, "tine-query"));
            for source in queries {
                let bounded = run_query_bounded(&graph, &source, usize::MAX, usize::MAX);
                let mut ids: Vec<String> = Vec::new();
                for group in &bounded.groups {
                    for block in &group.blocks {
                        ids.push(format!("{}#{}", group.page, block.id));
                    }
                }
                rows.push(format!(
                    "{label}\t{page}\t{:016x}\tblock\t{}\t{}\t{}",
                    fnv1a(source.as_bytes()),
                    bounded.total,
                    bounded.exceeded,
                    ids.join(","),
                ));
            }
        }
        rows.sort();
        if rows.is_empty() {
            println!("# no queries in {label}");
        }
        for row in rows {
            println!("{row}");
        }
        if let Some(dir) = scratch {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
